// The date token in a title editor: the one helper behind the draft row
// (`Row.tsx`) and the task surface's title (`TaskDialog.tsx`), so both
// recognise, show and drop a typed date phrase the same way.
//
// How it behaves, decided 2026-11-03:
//
//   - On every input the plain text is scanned (`findWhenPhrase`). The
//     moment a phrase parses it is wrapped in place in a non-editable
//     span, the token, that still reads as the words the user typed; the
//     resolved date is its hover text. The caret stays where it was,
//     outside the span.
//   - The token is a live reading of the text around it. Typing on after
//     it grows it ("friday" → "friday night" → "friday night at 9"),
//     typing ahead of it can grow it too ("next friday"), and a phrase
//     that stops parsing dissolves back into plain words (no rejection:
//     nothing was deleted). One token at most; another phrase elsewhere
//     in the text is left alone while one stands.
//   - Backspace with the caret just after the token (or Delete just
//     before it, or either key from inside it) removes the whole token,
//     words included, and rejects that phrase: typed again the same way
//     it stays plain text, while anything else, a different phrase or a
//     longer one, is taken as usual. A token deleted any other way (a
//     selection across it, select-all) is rejected the same, as soon as
//     the next input notices it gone. Rejections clear when the editor
//     is next focused.
//   - `read()` is the title without the token's words, and the token's
//     value. The editors save and mirror that, never the raw text.
//
// The host keeps calling `setLinkifiedText` / `textContent =` to reload
// the editor as it did; those drop the token's element, and `reset()`
// tells the helper so. The helper never reaches into the model: the
// hosts decide when the value is written, and both do so when the title
// is left (the row as it collapses, the task surface as it flushes),
// never while the phrase is still being typed.

import { createSignal } from "solid-js";
import {
  formatWhenBadge,
  formatWhenTime,
  todayStamp,
  whenDay,
} from "./format.tsx";

// Whether typed dates are recognised at all: a device preference like
// the time format, so it lives in a cookie beside it (`format.tsx`), on
// by default. Off, the editors never scan; a token already standing
// still commits.
const TYPED_DATES_COOKIE = "typeddates";

function readTypedDatesPref(): boolean {
  try {
    return !/(?:^|; )typeddates=0(?:;|$)/.test(document.cookie);
  } catch {
    return true;
  }
}

const [typedDatesPref, setTypedDatesSignal] = createSignal(readTypedDatesPref());
export { typedDatesPref };

export function setTypedDatesPref(on: boolean): void {
  setTypedDatesSignal(on);
  try {
    document.cookie = on
      ? `${TYPED_DATES_COOKIE}=;path=/;max-age=0;SameSite=Lax`
      : `${TYPED_DATES_COOKIE}=0;path=/;max-age=31536000;SameSite=Lax`;
  } catch {}
}
import { collapsedCaretOffset, placeCaretAtOffset } from "./linkify.ts";
import {
  collapseWhitespace,
  findWhenPhrase,
  phraseKey,
  type WhenPhrase,
} from "./whenParse.ts";

export interface WhenTokenValue {
  when: string;
  estimated: boolean;
  /** The phrase as typed. */
  phrase: string;
}

export interface WhenTokenRead {
  /** The title without the token's words (whitespace closed up when a
   *  token was cut out; otherwise the text as is, untrimmed). */
  text: string;
  token: WhenTokenValue | null;
}

export interface WhenTokens {
  /** Scan the editor after an input event; may wrap, reshape, dissolve
   *  or notice the loss of a token. */
  scan(): void;
  read(): WhenTokenRead;
  token(): WhenTokenValue | null;
  /** Handle a keydown; true when it consumed the key (the token was
   *  removed), in which case the host stops there. */
  onKeyDown(e: KeyboardEvent): boolean;
  /** The editor took focus: rejected phrases are forgotten. */
  onFocus(): void;
  /** The host replaced the editor's content; forget any token without
   *  rejecting (nothing was deleted by the user). */
  reset(): void;
}

export const WHEN_TOKEN_CLASS = "when-token";

/** Hover text for a token: the resolved day as the badges label it, the
 *  time in the app's format, a tilde ahead of an estimated time. */
export function whenTokenTitle(
  value: WhenTokenValue,
  now: number,
  locale: string,
  labels: { today: string; tomorrow: string; token: (label: string) => string },
): string {
  const day =
    formatWhenBadge(whenDay(value.when), todayStamp(now), labels, locale)?.label ??
    value.when;
  const time = formatWhenTime(value.when, locale);
  return labels.token(time ? `${day} ${value.estimated ? "~" : ""}${time}` : day);
}

export function createWhenTokens(
  el: HTMLElement,
  opts: {
    now: () => number;
    locale: () => string;
    /** Hover text for a token. */
    title: (value: WhenTokenValue) => string;
    /** Whether this editor scans at all right now (false for a done
     *  item: a date is a plan, and a done item has none to make). A
     *  standing token is unaffected. */
    enabled?: () => boolean;
    /** The token formed or changed (`value`), or went (null).
     *  `previous` is what stood before: null when the token is new. */
    onChange?: (value: WhenTokenValue | null, previous: WhenTokenValue | null) => void;
  },
): WhenTokens {
  let tokenEl: HTMLElement | null = null;
  const rejected = new Set<string>();

  const value = (): WhenTokenValue | null => {
    if (!tokenEl) return null;
    return {
      when: tokenEl.dataset.when ?? "",
      estimated: tokenEl.dataset.estimated !== undefined,
      phrase: tokenEl.textContent ?? "",
    };
  };

  const sameValue = (a: WhenTokenValue | null, b: WhenTokenValue | null) =>
    a?.when === b?.when && a?.estimated === b?.estimated && a?.phrase === b?.phrase;

  /** The token's character range in the plain text. */
  const tokenRange = (): { start: number; end: number } | null => {
    if (!tokenEl) return null;
    let start = 0;
    for (const node of Array.from(el.childNodes)) {
      if (node === tokenEl) break;
      start += node.textContent?.length ?? 0;
    }
    return { start, end: start + (tokenEl.textContent?.length ?? 0) };
  };

  /** Drop the token's element, keeping or discarding its words. */
  const takeOut = (keepWords: boolean) => {
    if (!tokenEl) return;
    if (keepWords) tokenEl.replaceWith(document.createTextNode(tokenEl.textContent ?? ""));
    else tokenEl.remove();
    tokenEl = null;
    el.normalize();
  };

  // The token is gone from the editor (deleted around the helper's
  // back): the user took it out, so its phrase is rejected.
  const noticeRemoved = () => {
    if (!tokenEl || el.contains(tokenEl)) return;
    const prev = value();
    rejected.add(phraseKey(prev?.phrase ?? ""));
    tokenEl = null;
    opts.onChange?.(null, prev);
  };

  const wrap = (phrase: WhenPhrase): boolean => {
    const start = pointAt(el, phrase.start);
    const end = pointAt(el, phrase.end);
    // Both ends must sit in the editor's own text, not inside a link:
    // a phrase can't be cut out of an anchor.
    if (!start || !end || start.node.parentNode !== el || end.node.parentNode !== el) {
      return false;
    }
    const range = document.createRange();
    range.setStart(start.node, start.offset);
    range.setEnd(end.node, end.offset);
    const span = document.createElement("span");
    span.className = WHEN_TOKEN_CLASS;
    span.contentEditable = "false";
    span.dataset.when = phrase.when;
    if (phrase.estimated) span.dataset.estimated = "";
    span.appendChild(range.extractContents());
    range.insertNode(span);
    tokenEl = span;
    span.title = opts.title(value()!);
    return true;
  };

  /** Put the caret back at character `offset`, never inside the token:
   *  an offset within or at the end of it lands just after the span, one
   *  at its start just before. */
  const placeCaret = (offset: number) => {
    const r = tokenRange();
    if (tokenEl && r && offset >= r.start && offset <= r.end) {
      const sel = window.getSelection();
      if (!sel) return;
      const range = document.createRange();
      if (offset === r.start) range.setStartBefore(tokenEl);
      else range.setStartAfter(tokenEl);
      range.collapse(true);
      sel.removeAllRanges();
      sel.addRange(range);
      return;
    }
    placeCaretAtOffset(el, offset);
  };

  const caretInsideToken = (): boolean => {
    const sel = window.getSelection();
    return Boolean(
      tokenEl && sel && sel.anchorNode && tokenEl.contains(sel.anchorNode),
    );
  };

  return {
    scan() {
      noticeRemoved();
      if (!typedDatesPref() || opts.enabled?.() === false) return;
      const text = el.textContent ?? "";
      const caret = collapsedCaretOffset(el);
      const prev = value();
      const anchor = tokenRange();
      const phrase = findWhenPhrase(text, {
        now: new Date(opts.now()),
        locale: opts.locale(),
        exclude: rejected,
        anchor,
      });
      if (anchor && phrase && phrase.start === anchor.start && phrase.end === anchor.end) {
        // Same words: refresh the value in case the day rolled over.
        if (tokenEl && (prev?.when !== phrase.when || prev?.estimated !== phrase.estimated)) {
          tokenEl.dataset.when = phrase.when;
          if (phrase.estimated) tokenEl.dataset.estimated = "";
          else delete tokenEl.dataset.estimated;
          tokenEl.title = opts.title(value()!);
          opts.onChange?.(value(), prev);
        }
        return;
      }
      if (anchor) takeOut(true);
      // A phrase that can't be wrapped (it runs into a link) stays plain.
      if (phrase) wrap(phrase);
      if (caret !== null && (anchor || tokenEl)) placeCaret(caret);
      const next = value();
      if (!sameValue(prev, next)) opts.onChange?.(next, prev);
    },

    read() {
      noticeRemoved();
      if (!tokenEl) return { text: el.textContent ?? "", token: null };
      let text = "";
      for (const node of Array.from(el.childNodes)) {
        if (node === tokenEl) text += " ";
        else text += node.textContent ?? "";
      }
      return { text: collapseWhitespace(text), token: value() };
    },

    token() {
      noticeRemoved();
      return value();
    },

    onKeyDown(e) {
      noticeRemoved();
      if (!tokenEl) return false;
      const sel = window.getSelection();
      if (!sel || sel.rangeCount === 0 || !sel.isCollapsed) return false;
      const { anchorNode: node, anchorOffset: off } = sel;
      if (!node) return false;
      const inside = caretInsideToken();
      if (e.key !== "Backspace" && e.key !== "Delete") {
        // A caret that has strayed inside the span (browsers differ on
        // where a click or arrow lands it) types after the token.
        if (inside && e.key.length === 1 && !e.metaKey && !e.ctrlKey && !e.altKey) {
          const r = tokenRange();
          if (r) placeCaret(r.end);
        }
        return false;
      }
      if (e.metaKey || e.ctrlKey || e.altKey) return false;
      const adjacent =
        inside ||
        (e.key === "Backspace"
          ? (node === el && el.childNodes[off - 1] === tokenEl) ||
            (node.previousSibling === tokenEl && off === 0)
          : (node === el && el.childNodes[off] === tokenEl) ||
            (node.nextSibling === tokenEl &&
              off === (node.textContent?.length ?? 0)));
      if (!adjacent) return false;
      e.preventDefault();
      const prev = value();
      const r = tokenRange();
      rejected.add(phraseKey(prev?.phrase ?? ""));
      takeOut(false);
      if (r) placeCaret(r.start);
      opts.onChange?.(null, prev);
      return true;
    },

    onFocus() {
      rejected.clear();
    },

    reset() {
      tokenEl = null;
      rejected.clear();
    },
  };
}

/** The text node and offset at character `offset` of `el`'s plain text.
 *  An offset on a node boundary resolves to the start of the later node,
 *  except at the very end, where it is the end of the last one. */
function pointAt(el: HTMLElement, offset: number): { node: Text; offset: number } | null {
  const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
  let remaining = offset;
  let last: Text | null = null;
  let node = walker.nextNode() as Text | null;
  while (node) {
    const len = node.data.length;
    if (remaining < len) return { node, offset: remaining };
    remaining -= len;
    last = node;
    node = walker.nextNode() as Text | null;
  }
  if (last && remaining === 0) return { node: last, offset: last.data.length };
  return null;
}
