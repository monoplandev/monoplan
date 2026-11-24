import { parseInternalUrl } from "./url.ts";

// Links in the contenteditable editors (row quick-entry, the task title
// and notes). The editors hold plain text only: URLs are painted with
// the CSS Custom Highlight API (`::highlight(link)` /
// `::highlight(link-hot)` in styles.css), never wrapped in anchors, so
// marking a URL as it is typed never touches the DOM and the browser's
// own undo history stays whole. A URL reads as text with a faint
// underline; with ⌘ / ⌥ held, the one under the pointer turns link blue
// and a click opens it.

// http(s) URLs, greedy up to the first whitespace/angle/quote. Trailing
// punctuation that isn't part of the URL (.,;:!?)]}'") is trimmed off so a
// sentence like "see https://x.com." doesn't swallow the full stop.
const URL_RE = /https?:\/\/[^\s<>"'`]+/g;
const URL_TRAIL_RE = /[.,;:!?)\]}'"]+$/;

type Link = { url: string; range: Range };

// Each editor's links, as live ranges over its text nodes.
const editors = new Map<HTMLElement, Link[]>();
// The highlight registry, or null where the API is missing (tests, an old
// browser): URLs then just don't show, and clicks still open them.
const highlights: { base: Highlight; hot: Highlight } | null =
  typeof CSS !== "undefined" && "highlights" in CSS && typeof Highlight !== "undefined"
    ? (() => {
        const base = new Highlight();
        const hot = new Highlight();
        hot.priority = 1;
        CSS.highlights.set("link", base);
        CSS.highlights.set("link-hot", hot);
        return { base, hot };
      })()
    : null;

// The URLs in `text` as [start, end) offsets.
function findUrls(text: string): { start: number; end: number; url: string }[] {
  const out: { start: number; end: number; url: string }[] = [];
  URL_RE.lastIndex = 0;
  let match: RegExpExecArray | null;
  while ((match = URL_RE.exec(text)) !== null) {
    let url = match[0];
    const trail = url.match(URL_TRAIL_RE);
    if (trail) url = url.slice(0, url.length - trail[0].length);
    if (url) out.push({ start: match.index, end: match.index + url.length, url });
  }
  return out;
}

// The text node and offset inside it at character `offset` of `el`'s text.
function pointAt(el: HTMLElement, offset: number): [Node, number] {
  const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
  let remaining = offset;
  let node = walker.nextNode() as Text | null;
  let last: Text | null = null;
  while (node) {
    if (remaining <= node.data.length) return [node, remaining];
    remaining -= node.data.length;
    last = node;
    node = walker.nextNode() as Text | null;
  }
  return last ? [last, last.data.length] : [el, 0];
}

// Re-read `el`'s text and re-mark its URLs. Call it after every edit: it
// only moves highlight ranges, so it costs nothing to the caret or undo.
export function refreshLinks(el: HTMLElement): void {
  for (const [other, links] of editors) {
    if (other !== el && !other.isConnected) {
      for (const l of links) dropRange(l.range);
      editors.delete(other);
    }
  }
  for (const l of editors.get(el) ?? []) dropRange(l.range);
  const text = el.textContent ?? "";
  const links: Link[] = findUrls(text).map(({ start, end, url }) => {
    const range = document.createRange();
    range.setStart(...pointAt(el, start));
    range.setEnd(...pointAt(el, end));
    highlights?.base.add(range);
    return { url, range };
  });
  if (links.length) editors.set(el, links);
  else editors.delete(el);
  updateHot();
}

// Stop marking `el`'s URLs (a row leaving edit mode).
export function clearLinks(el: HTMLElement): void {
  for (const l of editors.get(el) ?? []) dropRange(l.range);
  editors.delete(el);
  if (hot?.el === el) updateHot();
}

function dropRange(range: Range): void {
  highlights?.base.delete(range);
  highlights?.hot.delete(range);
}

// Put `text` into `el` as one plain text node and mark its URLs.
// `el.textContent` stays the string that gets saved back to the model.
export function setLinkifiedText(el: HTMLElement, text: string): void {
  el.replaceChildren();
  if (text) el.appendChild(document.createTextNode(text));
  refreshLinks(el);
}

// The link of `el` drawn under the viewport point (x, y), if any.
function linkAt(el: HTMLElement, x: number, y: number): Link | null {
  for (const link of editors.get(el) ?? []) {
    for (const r of link.range.getClientRects()) {
      if (x >= r.left && x <= r.right && y >= r.top && y <= r.bottom) return link;
    }
  }
  return null;
}

// The modifier that makes a URL a link: ⌘ or ⌥ (Ctrl stands in for ⌘ off
// the Mac; on the Mac a Ctrl-click is a right-click and never reaches
// `click`).
const isLinkModifier = (e: KeyboardEvent | MouseEvent): boolean =>
  e.metaKey || e.altKey || e.ctrlKey;

// The hover state: where the pointer last was, whether the modifier is
// held, and the link lit up because of both.
let pointer: { x: number; y: number; target: Element | null } | null = null;
let modHeld = false;
let hot: { el: HTMLElement; link: Link } | null = null;

function updateHot(): void {
  let next: { el: HTMLElement; link: Link } | null = null;
  if (modHeld && pointer?.target) {
    for (const el of editors.keys()) {
      if (!el.contains(pointer.target)) continue;
      const link = linkAt(el, pointer.x, pointer.y);
      if (link) next = { el, link };
      break;
    }
  }
  if (next?.link === hot?.link) return;
  if (hot) {
    highlights?.hot.delete(hot.link.range);
    hot.el.classList.remove("link-hot");
  }
  hot = next;
  if (hot) {
    highlights?.hot.add(hot.link.range);
    hot.el.classList.add("link-hot");
  }
}

if (typeof window !== "undefined") {
  const onKey = (e: KeyboardEvent) => {
    modHeld = isLinkModifier(e);
    updateHot();
  };
  window.addEventListener("keydown", onKey, true);
  window.addEventListener("keyup", onKey, true);
  window.addEventListener(
    "pointermove",
    (e) => {
      pointer = { x: e.clientX, y: e.clientY, target: e.target as Element | null };
      modHeld = isLinkModifier(e);
      updateHot();
    },
    { capture: true, passive: true },
  );
  // Scrolling moves text under a still pointer.
  window.addEventListener("scroll", () => updateHot(), { capture: true, passive: true });
  window.addEventListener("blur", () => {
    modHeld = false;
    updateHot();
  });
}

// Absolute character offset of the collapsed caret within `el`, or null when
// there is no selection inside el or it isn't collapsed. Used to detect
// "caret at start / end of field" for arrow-key navigation between editors.
export function collapsedCaretOffset(el: HTMLElement): number | null {
  const sel = window.getSelection();
  if (!sel || sel.rangeCount === 0 || !sel.isCollapsed) return null;
  const range = sel.getRangeAt(0);
  if (!el.contains(range.startContainer)) return null;
  const pre = range.cloneRange();
  pre.selectNodeContents(el);
  pre.setEnd(range.startContainer, range.startOffset);
  return pre.toString().length;
}

// Focus `el` and collapse the caret at character `offset` of its plain
// text (clamped to the content length). Walks the text nodes, so it
// works however the browser has split them while editing.
export function placeCaretAtOffset(el: HTMLElement, offset: number): void {
  el.focus();
  const sel = window.getSelection();
  if (!sel) return;
  const range = document.createRange();
  let remaining = Math.max(0, offset);
  const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
  let node: Node | null = walker.nextNode();
  let last: Text | null = null;
  while (node) {
    const t = node as Text;
    const len = t.data.length;
    if (remaining <= len) {
      range.setStart(t, remaining);
      range.collapse(true);
      sel.removeAllRanges();
      sel.addRange(range);
      return;
    }
    remaining -= len;
    last = t;
    node = walker.nextNode();
  }
  if (last) range.setStart(last, last.data.length);
  else range.selectNodeContents(el);
  range.collapse(last ? true : false);
  sel.removeAllRanges();
  sel.addRange(range);
}

// Focus `el` and collapse the caret to the end of its content.
export function placeCaretAtEnd(el: HTMLElement): void {
  el.focus();
  const sel = window.getSelection();
  const range = document.createRange();
  range.selectNodeContents(el);
  range.collapse(false);
  sel?.removeAllRanges();
  sel?.addRange(range);
}

// Focus `el` and collapse the caret to the start of its content.
export function placeCaretAtStart(el: HTMLElement): void {
  el.focus();
  const sel = window.getSelection();
  const range = document.createRange();
  range.selectNodeContents(el);
  range.collapse(true);
  sel?.removeAllRanges();
  sel?.addRange(range);
}

// A ⌘ / ⌥ click on a URL inside `el` opens it in a new tab, or, for one
// of the app's own item / list links (`spec/urls.md`), navigates the
// fragment in place (the workspace's popstate handler applies the route).
// A plain click is an ordinary click into the text. Returns true if a
// link was opened.
export function openLinkOnClick(e: MouseEvent, el: HTMLElement | undefined): boolean {
  if (!el || !isLinkModifier(e) || e.shiftKey) return false;
  const link = linkAt(el, e.clientX, e.clientY);
  if (!link) return false;
  e.preventDefault();
  if (parseInternalUrl(link.url) !== null) {
    // Same-document fragment navigation: pushes a history entry and
    // fires popstate. A no-op when the hash is already current.
    location.hash = new URL(link.url).hash;
  } else {
    window.open(link.url, "_blank", "noopener,noreferrer");
  }
  return true;
}
