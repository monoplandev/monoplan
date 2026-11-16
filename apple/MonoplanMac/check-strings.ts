#!/usr/bin/env bun
// Keeps each app's Localizable.xcstrings and its Swift sources in step.
// Nothing here uses Xcode's "extract strings" pass (the targets set
// SWIFT_EMIT_LOC_STRINGS = NO, so a build never rewrites a catalog), so
// this is it: every localizable call site in a target's sources must be
// a key in its catalog, every key must still be used, and every key
// must carry a Spanish translation whose format placeholders match the
// English. build-app.sh runs the Mac's before building; `bun run
// check:strings` runs them all, and `bun run check:strings mac` (or
// `ios`, `widget`) one. Exit 1 on any miss.
//
// A catalog belongs to a bundle, so there is one per target: the Mac
// app, the iOS app, and the iOS widget (an extension looks strings up
// in its own bundle, not the app's).
//
// A call site is `String(localized: "…")` (UIKit / AppKit and model
// strings) or a SwiftUI literal title: Text("…"), Button("…"), Toggle,
// Picker, Label, Section, ContentUnavailableView, …, and the modifiers
// that take one (.navigationTitle, .alert, .confirmationDialog,
// .accessibilityLabel, …; the lists are below). Those literals are
// LocalizedStringKeys, so they look up the same catalog with no call.
// A `\(…)` interpolation stands for one `%…` placeholder in the key
// (`%@` for strings, `%lld` for Int). Anything else (a ternary of
// literals, a String variable, a String parameter of our own views)
// is not seen here; wrap it in String(localized:) so it is. A literal
// with no letters of its own ("\(count)", "") has nothing to
// translate and is skipped; text that must stay as written goes in
// Text(verbatim:).
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";

const apple = join(import.meta.dir, "..");
const TARGETS: Record<string, { catalog: string; sources: string }> = {
  mac: { catalog: "MonoplanMac/Localizable.xcstrings", sources: "MonoplanMac/Sources" },
  ios: { catalog: "MonoplanIOS/Monoplan/Localizable.xcstrings", sources: "MonoplanIOS/Monoplan" },
  widget: { catalog: "MonoplanIOS/MonoplanWidget/Localizable.xcstrings", sources: "MonoplanIOS/MonoplanWidget" },
};
const VIEWS = [
  "Text", "Button", "Toggle", "Picker", "LabeledContent", "TextField", "SecureField", "DisclosureGroup",
  "Link", "Section", "Label", "NavigationLink", "ContentUnavailableView", "Menu", "DatePicker",
  "MultiDatePicker", "Tab",
];
const MODIFIERS = [
  "navigationTitle", "navigationSubtitle", "alert", "confirmationDialog", "accessibilityLabel",
  "accessibilityHint", "accessibilityValue", "help", "configurationDisplayName", "description",
];
const SWIFTUI = new RegExp(`(?:\\b(?:${VIEWS.join("|")})|\\.(?:${MODIFIERS.join("|")}))\\(\\s*"`, "g");
const LOCALIZED = /\bString\(localized:\s*"/g;
const PLACEHOLDER = /%(?:\d+\$)?(?:lld|llu|ld|lu|d|u|lf|f|@)/g;

type Site = { file: string; line: number; pattern: RegExp; display: string };

function swiftFiles(dir: string): string[] {
  const out: string[] = [];
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) out.push(...swiftFiles(p));
    else if (name.endsWith(".swift")) out.push(p);
  }
  return out;
}

/// Reads a Swift string literal starting just after its opening quote.
/// Returns the key as a regex (interpolations become a placeholder
/// pattern), a display form, and the index after the closing quote.
function readLiteral(src: string, start: number): { pattern: string; display: string; end: number } | null {
  let pattern = "";
  let display = "";
  let i = start;
  const escapeRe = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  while (i < src.length) {
    const c = src[i];
    if (c === '"') return { pattern, display, end: i + 1 };
    if (c === "\n") return null;
    if (c === "\\") {
      const n = src[i + 1];
      if (n === "(") {
        let depth = 1;
        let j = i + 2;
        while (j < src.length && depth > 0) {
          if (src[j] === "(") depth++;
          else if (src[j] === ")") depth--;
          j++;
        }
        pattern += PLACEHOLDER.source;
        display += "%…";
        i = j;
        continue;
      }
      const map: Record<string, string> = { n: "\n", t: "\t", '"': '"', "\\": "\\", "'": "'" };
      const ch = map[n] ?? n;
      pattern += escapeRe(ch);
      display += ch;
      i += 2;
      continue;
    }
    pattern += escapeRe(c);
    display += c;
    i++;
  }
  return null;
}

function sites(file: string): Site[] {
  const text = readFileSync(file, "utf8");
  // Drop whole-line comments so a `Text("…")` in prose is not a call.
  const src = text.replace(/^\s*\/\/.*$/gm, (m) => " ".repeat(m.length));
  const found: Site[] = [];
  for (const re of [SWIFTUI, LOCALIZED]) {
    re.lastIndex = 0;
    let m: RegExpExecArray | null;
    while ((m = re.exec(src))) {
      const lit = readLiteral(src, m.index + m[0].length);
      // Nothing of its own to translate: "", "\(count)", "\(a) \(b)".
      if (!lit || !/\p{L}/u.test(lit.display.replaceAll("%…", ""))) continue;
      const line = src.slice(0, m.index).split("\n").length;
      found.push({ file, line, pattern: new RegExp(`^${lit.pattern}$`), display: lit.display });
    }
  }
  return found;
}

type Catalog = {
  sourceLanguage: string;
  strings: Record<string, { localizations?: Record<string, { stringUnit?: { state: string; value: string } }> }>;
};

function check(name: string): { errors: string[]; count: number } {
  const target = TARGETS[name];
  const catalogPath = join(apple, target.catalog);
  const errors: string[] = [];
  if (!existsSync(catalogPath)) return { errors: [`${target.catalog}: missing`], count: 0 };
  const catalog: Catalog = JSON.parse(readFileSync(catalogPath, "utf8"));
  const keys = Object.keys(catalog.strings);
  const used = new Set<string>();
  const rel = (f: string) => f.slice(apple.length + 1);

  for (const file of swiftFiles(join(apple, target.sources))) {
    for (const s of sites(file)) {
      const matches = keys.filter((k) => s.pattern.test(k));
      if (matches.length === 0) {
        errors.push(`${rel(s.file)}:${s.line}: "${s.display}" is not in ${target.catalog}`);
      } else if (matches.length > 1) {
        errors.push(`${rel(s.file)}:${s.line}: "${s.display}" matches several keys: ${matches.map((k) => `"${k}"`).join(", ")}`);
      } else {
        used.add(matches[0]);
      }
    }
  }

  for (const key of keys) {
    if (!used.has(key)) errors.push(`${target.catalog}: "${key}" is not used by any source`);
    const es = catalog.strings[key].localizations?.es?.stringUnit;
    if (!es || es.state !== "translated" || es.value.trim() === "") {
      errors.push(`${target.catalog}: "${key}" has no Spanish translation`);
      continue;
    }
    const want = (key.match(PLACEHOLDER) ?? []).sort().join(" ");
    const have = (es.value.match(PLACEHOLDER) ?? []).sort().join(" ");
    if (want !== have) errors.push(`${target.catalog}: "${key}" → "${es.value}": placeholders differ`);
  }
  return { errors, count: keys.length };
}

const asked = process.argv.slice(2);
const unknown = asked.filter((a) => !(a in TARGETS));
if (unknown.length) {
  console.error(`check-strings: unknown target ${unknown.join(", ")} (have: ${Object.keys(TARGETS).join(", ")})`);
  process.exit(2);
}
let problems = 0;
for (const name of asked.length ? asked : Object.keys(TARGETS)) {
  const { errors, count } = check(name);
  for (const e of errors) console.error(`error: ${e}`);
  problems += errors.length;
  if (!errors.length) console.log(`check-strings: ${name}: ${count} strings, all used and translated`);
}
if (problems) {
  console.error(`check-strings: ${problems} problem(s)`);
  process.exit(1);
}
