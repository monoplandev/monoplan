#!/usr/bin/env bun
// Keeps Localizable.xcstrings and the Swift sources in step. SwiftPM has
// no Xcode "extract strings" pass, so this is it: every localizable
// call site in Sources/ must be a key in the catalog, every key must
// still be used, and every key must carry a Spanish translation whose
// format placeholders match the English. build-app.sh runs it before
// building; `bun run check:strings` runs it alone. Exit 1 on any miss.
//
// A call site is `String(localized: "…")` (AppKit and model strings, in
// both targets) or a SwiftUI literal title: Text("…"), Button("…"),
// Toggle, LabeledContent, TextField, SecureField, DisclosureGroup, Link,
// Section, Label. Those literals are LocalizedStringKeys, so they look
// up the same catalog with no call. A `\(…)` interpolation stands for
// one `%…` placeholder in the key (`%@` for strings, `%lld` for Int).
// Anything else (a ternary of literals, a String variable) does *not*
// localize; wrap it in String(localized:) so it shows up here.
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";

const root = import.meta.dir;
const catalogPath = join(root, "Localizable.xcstrings");
const sourceRoot = join(root, "Sources");
const SWIFTUI = /\b(?:Text|Button|Toggle|LabeledContent|TextField|SecureField|DisclosureGroup|Link|Section|Label)\(\s*"/g;
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
      if (!lit || lit.display === "") continue;
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

const catalog: Catalog = JSON.parse(readFileSync(catalogPath, "utf8"));
const keys = Object.keys(catalog.strings);
const used = new Set<string>();
const errors: string[] = [];
const rel = (f: string) => f.slice(root.length + 1);

for (const file of swiftFiles(sourceRoot)) {
  for (const s of sites(file)) {
    const matches = keys.filter((k) => s.pattern.test(k));
    if (matches.length === 0) {
      errors.push(`${rel(s.file)}:${s.line}: "${s.display}" is not in Localizable.xcstrings`);
    } else if (matches.length > 1) {
      errors.push(`${rel(s.file)}:${s.line}: "${s.display}" matches several keys: ${matches.map((k) => `"${k}"`).join(", ")}`);
    } else {
      used.add(matches[0]);
    }
  }
}

for (const key of keys) {
  if (!used.has(key)) errors.push(`Localizable.xcstrings: "${key}" is not used by any source`);
  const es = catalog.strings[key].localizations?.es?.stringUnit;
  if (!es || es.state !== "translated" || es.value.trim() === "") {
    errors.push(`Localizable.xcstrings: "${key}" has no Spanish translation`);
    continue;
  }
  const want = (key.match(PLACEHOLDER) ?? []).sort().join(" ");
  const have = (es.value.match(PLACEHOLDER) ?? []).sort().join(" ");
  if (want !== have) errors.push(`Localizable.xcstrings: "${key}" → "${es.value}": placeholders differ`);
}

if (errors.length) {
  for (const e of errors) console.error(`error: ${e}`);
  console.error(`check-strings: ${errors.length} problem(s)`);
  process.exit(1);
}
console.log(`check-strings: ${keys.length} strings, all used and translated`);
