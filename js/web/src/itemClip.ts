// Item clipboard: what a copied selection puts on the system clipboard and
// how a paste reads it back.
//
// A clipboard entry is a bag of MIME-typed payloads, not one string, so a
// single copy carries three representations and each consumer picks the
// one it understands:
//
// - `text/plain`: bare titles, one per line. This is what lands in Slack,
//   a commit message, or a text editor. Pasted back into Monoplan it still
//   works through the plain line splitter, just without the details.
// - `text/html`: a real `<ul>` of titles, so rich-text apps render a list,
//   with the full structured payload stashed as a `data-monoplan`
//   attribute on the wrapper. Every browser writes and reads `text/html`
//   through both the async clipboard API and the copy / paste events, so
//   this is the portable carrier for a structured round-trip.
// - `application/x-monoplan+json`: the same payload as a first-class type.
//   Only settable on the synchronous `copy` event; Chromium and Firefox
//   round-trip it freely and Safari lets only the same origin read it
//   back, which is exactly the reader we have.
//
// Paste prefers the custom type, then the html payload, then plain lines.
// The payload carries the fields that define a clone (title, notes, the
// dates, duration, workflow state) and nothing that ties an item to where
// it came from: no id, no list, no timestamps. A structured paste is a
// duplicate into the target view.
import { type ItemView, type WorkflowState } from "./sync/store.ts";

export const ITEM_CLIP_TYPE = "application/x-monoplan+json";

/** Temporary: log every copy and paste to the console while the
 *  notes-not-copied report is being chased. */
const CLIP_DEBUG = true;

/** One copied item: the clone-defining fields of `ItemView`. */
export interface ClipItem {
  text: string;
  notes?: string;
  deadline?: string;
  when?: string;
  duration?: number;
  state: WorkflowState;
}

/** The serialised payload. Bump `v` on any incompatible shape change so an
 *  old tab's copy is dropped (falls back to plain text) instead of
 *  misread. */
export interface ItemClip {
  v: 1;
  items: ClipItem[];
}

const STATES: ReadonlySet<string> = new Set<WorkflowState>([
  "backlog",
  "todo",
  "in_progress",
  "review",
  "done",
]);

export const toClipItem = (it: ItemView): ClipItem => {
  const out: ClipItem = { text: it.text, state: it.state };
  if (it.notes) out.notes = it.notes;
  if (it.deadline) out.deadline = it.deadline;
  if (it.when) out.when = it.when;
  if (it.when && it.duration) out.duration = it.duration;
  return out;
};

const escapeHtml = (s: string): string =>
  s
    .replace(/&/g, "&amp;")
    .replace(/"/g, "&quot;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");

const unescapeHtml = (s: string): string =>
  s.replace(/&(#x[0-9a-f]+|#\d+|[a-z]+);/gi, (m, ent: string) => {
    if (ent[0] === "#") {
      const code =
        ent[1] === "x" || ent[1] === "X"
          ? parseInt(ent.slice(2), 16)
          : parseInt(ent.slice(1), 10);
      return Number.isFinite(code) ? String.fromCodePoint(code) : m;
    }
    switch (ent.toLowerCase()) {
      case "amp":
        return "&";
      case "quot":
        return '"';
      case "lt":
        return "<";
      case "gt":
        return ">";
      case "apos":
        return "'";
      case "nbsp":
        return " ";
      default:
        return m;
    }
  });

/** The three clipboard representations of a copied block. */
export interface ClipPayloads {
  text: string;
  html: string;
  json: string;
}

export const renderClip = (items: readonly ClipItem[]): ClipPayloads => {
  const clip: ItemClip = { v: 1, items: [...items] };
  const json = JSON.stringify(clip);
  const text = items.map((it) => it.text).join("\n");
  const html =
    `<ul data-monoplan="${escapeHtml(json)}">` +
    items.map((it) => `<li>${escapeHtml(it.text)}</li>`).join("") +
    `</ul>`;
  return { text, html, json };
};

const parseClip = (raw: string): ClipItem[] | null => {
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return null;
  }
  if (typeof parsed !== "object" || parsed === null) return null;
  const clip = parsed as Partial<ItemClip>;
  if (clip.v !== 1 || !Array.isArray(clip.items)) return null;
  const items: ClipItem[] = [];
  for (const entry of clip.items as unknown[]) {
    if (typeof entry !== "object" || entry === null) return null;
    const e = entry as Record<string, unknown>;
    if (typeof e.text !== "string") return null;
    const item: ClipItem = {
      text: e.text,
      state:
        typeof e.state === "string" && STATES.has(e.state)
          ? (e.state as WorkflowState)
          : "backlog",
    };
    if (typeof e.notes === "string" && e.notes) item.notes = e.notes;
    if (typeof e.deadline === "string" && e.deadline)
      item.deadline = e.deadline;
    if (typeof e.when === "string" && e.when) item.when = e.when;
    if (
      item.when &&
      typeof e.duration === "number" &&
      Number.isInteger(e.duration) &&
      e.duration > 0
    )
      item.duration = e.duration;
    items.push(item);
  }
  return items;
};

/** Pull the structured payload back out of a `text/html` clipboard
 *  string. Regex rather than DOMParser: the writer is ours, so the
 *  attribute shape is known, and this keeps the reader DOM-free. Browser
 *  sanitisers re-serialise attributes but keep the standard entity
 *  escapes, which `unescapeHtml` reverses. */
export const clipFromHtml = (html: string): ClipItem[] | null => {
  const m = /data-monoplan="([^"]*)"/.exec(html);
  if (!m) return null;
  return parseClip(unescapeHtml(m[1]));
};

/** Read a paste: the custom type first, then the html payload. `null`
 *  means no structured payload; fall back to plain lines. */
export const readClip = (dt: DataTransfer | null): ClipItem[] | null => {
  if (!dt) return null;
  const items = readClipInner(dt);
  if (CLIP_DEBUG) {
    const trunc = (s: string) => (s.length > 3000 ? `${s.slice(0, 3000)}…` : s);
    console.log("[clip] paste", {
      types: [...dt.types],
      custom: trunc(dt.getData(ITEM_CLIP_TYPE)),
      html: trunc(dt.getData("text/html")),
      text: trunc(dt.getData("text/plain")),
      parsed: items,
    });
  }
  return items;
};

const readClipInner = (dt: DataTransfer): ClipItem[] | null => {
  const custom = dt.getData(ITEM_CLIP_TYPE);
  if (custom) {
    const items = parseClip(custom);
    if (items) return items;
  }
  const html = dt.getData("text/html");
  if (html) return clipFromHtml(html);
  return null;
};

/** Write a copied block. Given the `copy` event's `DataTransfer`, set all
 *  three types synchronously (the caller must `preventDefault`). Without
 *  one (a context menu, the command palette) go through the async API
 *  with the two standard types; the html payload still carries the
 *  structure. Falls back to plain text where `ClipboardItem` is missing. */
export const writeClip = (
  items: readonly ClipItem[],
  dt?: DataTransfer | null,
): void => {
  const p = renderClip(items);
  if (dt) {
    dt.setData("text/plain", p.text);
    dt.setData("text/html", p.html);
    dt.setData(ITEM_CLIP_TYPE, p.json);
    if (CLIP_DEBUG)
      console.log("[clip] wrote via copy event", { types: [...dt.types], ...p });
    return;
  }
  if (typeof ClipboardItem === "undefined" || !navigator.clipboard.write) {
    if (CLIP_DEBUG) console.log("[clip] wrote via writeText (no ClipboardItem)", p);
    void navigator.clipboard.writeText(p.text);
    return;
  }
  if (CLIP_DEBUG) console.log("[clip] writing via async ClipboardItem", p);
  const entry = new ClipboardItem({
    "text/plain": new Blob([p.text], { type: "text/plain" }),
    "text/html": new Blob([p.html], { type: "text/html" }),
  });
  navigator.clipboard.write([entry]).catch(() => {
    void navigator.clipboard.writeText(p.text);
  });
};
