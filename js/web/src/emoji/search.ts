// Search over the vendored emoji dataset.
//
// Deliberately *not* an inverted index like the workspace search in core
// (`core/src/search.rs`). That one indexes a live, mutating corpus of
// unbounded size and has to track every doc change. This corpus is ~1900
// fixed records loaded once, so a linear scan over pre-tokenised fields is
// both faster to build and far less code — no postings, no prefix bucket,
// nothing to invalidate.
//
// Tokenisation is the core tokenizer, re-exported through `../search.ts`,
// so the folding rules in spec/search.md (case, accents, punctuation)
// apply identically here.

import type { Emoji } from "./data.ts";
import { tokenize } from "../search.ts";

/** Pre-tokenised record. Built once per index; the token arrays are what
 *  the per-keystroke scan actually walks. */
interface EmojiDoc {
  emoji: Emoji;
  /** Position in the source dataset — emojibase `order`, which is roughly
   *  canonical/frequency ordering. The final tie-break. */
  order: number;
  /** Every full name this emoji goes by — its annotation plus each shortcode
   *  — normalised to space-separated tokens. Matched against the whole query
   *  as a unit; see `SCORE_WHOLE_NAME`. */
  names: string[];
  nameTokens: string[];
  shortcodeTokens: string[];
  tagTokens: string[];
}

export interface EmojiIndex {
  /** Emoji in a group, in dataset order. Never includes unsupported emoji. */
  byGroup(groupId: number): Emoji[];
  /** Look up by literal grapheme — resolves stored icons and recents back to
   *  their metadata. Undefined for anything not in (or filtered out of) the
   *  dataset, which is expected: icons are stored verbatim and a user may
   *  have pasted something arbitrary. */
  get(emoji: string): Emoji | undefined;
  /** Ranked matches. Empty query returns `[]` — the caller shows groups. */
  query(input: string, limit?: number): Emoji[];
}

// Naming the thing outright beats every per-token signal combined. Without
// this, per-token scoring alone can't separate 🔥 ("fire") from ❤️‍🔥
// ("heart on fire", shortcode `heart_on_fire`) for the query "fire": both
// carry an exact `fire` token, and the tie falls to dataset order, which puts
// the wrong one first. Awarded once per doc, not per token.
const SCORE_WHOLE_NAME = 5000;

// Per-token weights, highest signal first. A shortcode is the closest thing
// to a canonical name a user types deliberately (":tada:"), so it outranks
// the CLDR annotation; tags are the loosest signal and rank last. Exact beats
// prefix within each field.
const SCORE_SHORTCODE_EXACT = 1000;
const SCORE_SHORTCODE_PREFIX = 300;
const SCORE_NAME_EXACT = 200;
const SCORE_NAME_PREFIX = 60;
const SCORE_TAG_EXACT = 20;
const SCORE_TAG_PREFIX = 5;

/** Best score for one query token against one field's tokens, or 0 for no
 *  match. Prefix matching applies to every token, not just the last one —
 *  unlike the workspace index, there's no postings list forcing earlier
 *  tokens to be exact, and emoji queries are typically short and truncated
 *  ("smi", "arr"). */
function scoreField(
  queryToken: string,
  fieldTokens: readonly string[],
  exactScore: number,
  prefixScore: number,
): number {
  let best = 0;
  for (const t of fieldTokens) {
    if (t === queryToken) return exactScore;
    if (prefixScore > best && t.startsWith(queryToken)) best = prefixScore;
  }
  return best;
}

/** Build an index over `emoji`, dropping anything newer than `maxVersion`
 *  (see `support.ts` — emoji this device's fonts can't draw would otherwise
 *  show as tofu). Pass `Infinity` to keep everything. */
export function createEmojiIndex(
  emoji: readonly Emoji[],
  maxVersion: number,
): EmojiIndex {
  const docs: EmojiDoc[] = [];
  const groups = new Map<number, Emoji[]>();
  const byChar = new Map<string, Emoji>();

  for (const e of emoji) {
    if (e.version > maxVersion) continue;
    byChar.set(e.emoji, e);
    const shortcodes = e.shortcodes.length > 0 ? e.shortcodes.split(" ") : [];
    docs.push({
      emoji: e,
      order: docs.length,
      // Normalise each name through the tokenizer and rejoin, so a shortcode
      // ("heart_on_fire") and a typed query ("heart on fire") meet at the
      // same form.
      names: [e.annotation, ...shortcodes].map((n) => tokenize(n).join(" ")),
      nameTokens: tokenize(e.annotation),
      // Shortcodes are `snake_case`; the shared tokenizer splits on
      // non-alphanumerics, so "grinning_face" indexes as two tokens and a
      // query for either half hits.
      shortcodeTokens: tokenize(e.shortcodes),
      tagTokens: tokenize(e.tags),
    });
    let bucket = groups.get(e.group);
    if (!bucket) groups.set(e.group, (bucket = []));
    bucket.push(e);
  }

  function byGroup(groupId: number): Emoji[] {
    return groups.get(groupId) ?? [];
  }

  function get(char: string): Emoji | undefined {
    return byChar.get(char);
  }

  function query(input: string, limit = 60): Emoji[] {
    const tokens = tokenize(input);
    if (tokens.length === 0) return [];
    const whole = tokens.join(" ");

    const scored: { doc: EmojiDoc; score: number }[] = [];
    for (const doc of docs) {
      let total = doc.names.includes(whole) ? SCORE_WHOLE_NAME : 0;
      // AND across query tokens: every token must hit some field.
      for (const qt of tokens) {
        const best = Math.max(
          scoreField(
            qt,
            doc.shortcodeTokens,
            SCORE_SHORTCODE_EXACT,
            SCORE_SHORTCODE_PREFIX,
          ),
          scoreField(qt, doc.nameTokens, SCORE_NAME_EXACT, SCORE_NAME_PREFIX),
          scoreField(qt, doc.tagTokens, SCORE_TAG_EXACT, SCORE_TAG_PREFIX),
        );
        if (best === 0) {
          total = 0;
          break;
        }
        total += best;
      }
      if (total > 0) scored.push({ doc, score: total });
    }

    scored.sort((a, b) => {
      if (a.score !== b.score) return b.score - a.score;
      // On a tie, the shorter name is the more specific match: "arrow" scores
      // the same exact-token hit against "up arrow" and "heart with arrow",
      // and the former is what was meant.
      if (a.doc.nameTokens.length !== b.doc.nameTokens.length) {
        return a.doc.nameTokens.length - b.doc.nameTokens.length;
      }
      return a.doc.order - b.doc.order;
    });
    return scored.slice(0, limit).map((s) => s.doc.emoji);
  }

  return { byGroup, get, query };
}
