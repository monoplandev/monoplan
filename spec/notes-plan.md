# Notes as LoroText: plan

**Status: Phases 0, 1 and 2 done 2026-09-08, Phases 3-4 not built.** Moves `item.notes` from a whole-string LWW
register to a mergeable `LoroText` child container, and lays out the path
from there to rich text with images. Companion to `sharing-plan.md` ("Text
fields must be mergeable before sharing"), which this plan supersedes for the
storage question.

Research date: 2026-09-04, amended 2026-09-08 after a source read of
`loro` / `loro-internal` 1.13.9. Loro versions at that date: Rust crate
`loro` 1.13.9 (2026-08-01), npm `loro-crdt` 1.15.1 (2026-08-29). Monoplan
pinned `loro = "1.10"` in `Cargo.toml` (now `"1.16"`, Phase 0); `Cargo.lock` resolved 1.13.9
at research time and 1.16.0 after the bump.

## Decisions in one screen

| Question | Decision |
|---|---|
| Separate doc or same doc? | **Same doc.** `notes` becomes a `LoroText` child of the item map at `items/<id>/notes`. No Monoplan-managed root container (Loro itself backs the child with a derived-name root, see the correction below), no second doc. |
| Lazy creation | **Yes, via Loro mergeable containers** (`LoroMap::ensure_mergeable_text`, Rust 1.13.1+). Created on first write; concurrent first writes on two devices merge into one text. Items without notes carry nothing. |
| Editor connector | **None fits as-is.** Every Loro editor binding needs a JS `LoroDoc`; Monoplan's doc lives in Rust wasm. We write a thin delta bridge across the wasm boundary instead. |
| Rich text model | **Flat rich text in one `LoroText`** (Quill delta model: inline marks, line formats as attributes on `\n`). Not a ProseMirror node tree. |
| Images | **By reference**, never bytes in the CRDT. A `U+FFFC` placeholder character carrying an `image` attribute that names an encrypted attachment. Attachments are a new dumb server surface (own spec, later phase). |
| `text` field | **Stays a string register.** A title is one short phrase rewritten whole, so a character merge of two concurrent rewrites interleaves them into nonsense; LWW gives one clean title. Also saves a container per item. Decided 2026-09-08 (see "Why `text` stays a register"). |
| Schema | **v3 to v4**, clean break, export / wipe / import per `data-model.md` policy. |

## 1. Storage

### Why the same doc

- One sync engine, one WAL, one compaction horizon, one DEK. A separate
  notes doc per item is the multi-doc substrate from `sharing-plan.md`
  Phases 1-5 (about six days) plus per-doc frontier and snapshot management
  for thousands of tiny docs. Nothing about notes needs it.
- Notes are short. Monoplan holds next steps, not plans (`subtasks.md`); a
  note is context for one item, not a long-form document. The failure mode
  that justifies a separate doc (one huge text dominating snapshot size) is
  outside the product.
- Sharing later shares the item and its notes together for free.
- Diff translation already routes nested containers under `items` by path
  root (`classify_captured_diff`, `core/src/doc.rs`); a text child slots in
  with one new arm.

### Why a child of the item map, not a `notes/<id>` root container

A per-item root container (`doc.get_text("notes/<id>")`) is op-free and
merges concurrent creation, and Monoplan already uses that pattern for
`order/<list-id>`. It was the fallback if mergeable containers had not
shipped. Rejected because the item's fields would live in two places for
reads, hashes, export, and duplication. The mergeable child gives the same
merge guarantee inside the item.

Correction from Phase 1 (2026-09-08): the earlier draft also argued that a
root per item "can never be removed from the doc". That is equally true of
the mergeable child, because Loro implements it *as* a root container with
a derived name (`🤝:<hash>>notes`, `loro_common::MERGEABLE_NAMESPACE_PREFIX`)
referenced by a binary marker in the map slot. Hard-deleting an item hides
the text but its state stays in the doc either way. The one-place-for-reads
argument stands on its own.

### Mergeable containers (the "lazy creation merge" feature)

Shipped 2026-06 (Loro blog "Mergeable Containers", PR #991, issue #759):

- `LoroMap::ensure_mergeable_text(key) -> LoroText` and siblings for map,
  list, movable list, tree, counter. Rust crate 1.13.1+, npm 1.13.0+.
- The child gets a deterministic id derived from `(parent, key, type)`, so
  two peers calling `ensure_mergeable_text("notes")` concurrently produce
  the same container and their inserts interleave as one text.
- First call writes one small marker op into the map slot; repeat calls are
  idempotent and write nothing. Deleting the map key removes the marker
  (LWW) but the child's state stays in the doc, and the Loro docs are
  explicit that a later `ensure` on the same key "writes the ref back and
  resurfaces the preserved child state". **Consequence: never clear notes
  by deleting the key.** A user who cleared `hello` and later typed `h`
  would get `hhello` once edits are deltas (Phase 2). Clear notes by
  deleting the text content (`delete(0, len)`); the key and marker stay.
- `get_or_create_container` is now deprecated in favour of this.
- Contrast with today's `insert_container` on the same key from two peers:
  LWW on the map key, one text hidden. The loser is recoverable only by
  container id (`is_deleted() == true`), which is not a UX.

Consequence: **new items do not get a notes container at creation.** The
container is created on first notes write on whichever device writes first,
and the merge is safe. This is the one place the eager-create workaround
(create at item creation, same commit) is unnecessary.

### Item table after this plan

| Field | v3 | v4 |
|---|---|---|
| `text` | string register | string register (unchanged) |
| `notes` | string register | mergeable `LoroText`, absent until first write, empty (not absent) after a clear, may carry marks |

Everything else unchanged. `data-model.md` gets the `notes` row plus a
"v3 to v4" paragraph under schema versioning.

### Why `text` stays a register

An earlier draft converted `text` in the same cutover ("one schema bump
instead of two"). Dropped 2026-09-08:

- A title is rewritten whole, not edited in place. Two concurrent
  `update` calls each diff to "delete most of it, insert a new phrase",
  and Loro merges those into an interleaving ("Buy oat milk" and "Get
  milk from Coles" becoming "Get Buy oat milk from Coles"). LWW keeps one
  clean title; the dropped edit is a visible, comprehensible loss.
  Character merge suits notes because notes are longer and concurrent
  edits usually touch different regions.
- Every item would carry a text container plus its marker op, and every
  item view would call `to_string()` on it. Notes pay that only on items
  that have notes. Boot replay (`tui-plan.md`) is sensitive to op and
  container counts per item.
- The never-empty invariant is one line in `edit_item_text` today. With a
  text container it becomes a property of the merged result rather than of
  any single write.
- Sharing does not need it. The `sharing-plan.md` "text fields must be
  mergeable" line is about not silently dropping edits; losing one
  concurrent title rewrite to LWW is the acceptable case.
- No second bump is implied: `text` stays a register on purpose, so there
  is no deferred migration.

### Loro version bump

Root `Cargo.toml`: `loro = "1.16"` (Phase 0 first raised the floor to 1.13
against the existing 1.13.9 lock, then bumped to 1.16.0 on 2026-09-08;
wasm build and the full workspace test suite pass on 1.16.0). Verify
`bun run test` and `bun run build:wasm` after any further bump. Note the Rust crate lags the npm package: the
1.14 / 1.15 fixes (atomic `import_batch`, O(n^2) styled-text read fix,
redundant mark dedupe) are not on crates.io yet. The styled-read fix matters
for rich text with many marks; notes are short, so this is a watch item,
not a blocker. Update 2026-09-08: the Rust crate jumped to 1.16.0 on
2026-09-06 (no 1.14 / 1.15 on crates.io) and still exposes every API this
plan uses; the floor is now 1.16. Whether the styled-read fix is included
is unverified and only matters once Phase 3 adds marks.

### Index units

Loro has three index systems for one text: Unicode scalars, UTF-8 bytes,
and UTF-16 code units. The unqualified methods and the positions inside
`Diff::Text` events use the "event" unit, which is UTF-16 only when
`loro-internal`'s `wasm` feature is on. That feature is not forwarded by
the public `loro` crate (its features are `counter`, `jsonpath`,
`logging`); it exists for `loro-wasm`, the crate behind the `loro-crdt`
npm package. Monoplan depends on `loro`, so in every Monoplan build, the
browser wasm included, **core text indices are Unicode scalars**.

Browser editors count UTF-16 code units. Scalars and UTF-16 agree until
the first astral-plane character (emoji and friends), which is one scalar
but two UTF-16 units; after it every editor position is off and Loro
inserts in the wrong place, deletes a neighbour, or rejects the op as out
of bounds. Integers cross the wasm boundary untouched (only strings are
re-encoded, to UTF-8), so a UTF-16 offset lands in Rust still reading as
the same number. The bridge converts in one place, on the Rust side:

- Inbound, Phase 2 (no marks): do not use `apply_delta`. Validate the
  whole delta against the current text first (built: `validate_utf16_delta`),
  then walk it with a UTF-16 cursor and call `insert_utf16` / `delete_utf16` directly.
  Loro validates every boundary (`UTF16InUnicodeCodePoint` on a split
  surrogate pair) and resolves the index through the rope's cached
  counts. `convert_pos` on an attached container flattens the text to a
  `String` and walks it linearly for the source unit, so a convert-then-
  `apply_delta` path is both slower and more code to get wrong.
- Inbound, Phase 3 (marks): `apply_delta` is needed for attributes and
  only speaks the event unit (scalars here). A Quill delta is expressed
  against the pre-change text, so convert once, before applying: walk the
  delta over the pre-change string with a UTF-16 cursor and a scalar
  cursor, rewrite each retain / delete count, count each insert string
  directly, then call `apply_delta` on the rewritten delta.
- Outbound: `Diff::Text` deltas arrive in scalars. `TextDelta::Delete` is
  only a count and the deleted characters are gone from the post-change
  text, so **conversion must run against the pre-change text**. The bridge
  keeps a shadow `String` per subscribed item: convert each retain / delete
  count against the shadow at the running cursor, count insert strings
  directly, then apply the scalar delta to the shadow. Emit
  `itemNotesDelta` in UTF-16.
- The JS adaptor never handles units. It should send editor changes only
  after `compositionend`; a lone surrogate mid-composition becomes
  `U+FFFD` in the wasm string encoder and would leave the editor and the
  CRDT holding different text of the same width.

Phase 1 is unaffected because `update` takes a whole string and diffs it
internally. The unit question arrives with the delta bridge in Phase 2.

## 2. Core changes (Phase 1, plain text, merge-correct)

Goal: character-level merges with zero UI change. This is the
`sharing-plan.md` pre-flight item, done properly.

Scope of the fix: **offline and cross-device edits merge; an open dialog
still overwrites.** The dialog writes on close or target switch
(`flush` in `TaskDialog.tsx`), never per keystroke, and does not reload
notes while open. `update(local)` makes the text equal the local string,
so a remote edit that arrives while the dialog is open is diffed away,
exactly as today. Two devices editing offline and syncing later are
merged character by character, because each side's ops are concurrent.
The live-under-caret case is Phase 2.

- `edit_item_notes(id, notes: &str)`: `ensure_mergeable_text(KEY_NOTES)`
  then `text.update(notes, UpdateOptions::default())` (Myers diff; the
  default has no timeout, so the `UpdateTimeoutError` arm cannot fire).
  Removed 2026-09-23 once every caller had moved to `apply_notes_delta`;
  see the Phase 2 note below.
  Empty string: `update("")`, which deletes the content and keeps the key,
  never a key delete (see the resurface note under Mergeable containers).
  Signature unchanged, so CLI, wasm bindings, import, and duplicate-list
  callers are untouched.
- `edit_item_text`, item creation, and the row renderer: untouched.
  `text` stays a string register.
- Reads: `item_view` reads `LoroText::to_string()` for `notes`
  (`read_text_or_string` helper: accept a text container; a stray string
  value is a v3 leftover and, per the clean-break policy, is not read).
- Diff classifier: a `LoroDiff::Text` whose event path has the shape
  `[(items, _), (item_map, Key(id)), (text, Key(key))]` becomes
  `CapturedDiff::ItemMap { container: item_map, keys: {key} }`. The
  target container cannot be used for routing because a mergeable child
  is a root container with a derived name (see the correction above), so
  the classifier now takes the whole path (each entry pairs a container
  with its index in its parent). Before Phase 1 it fell through to
  `Opaque`, which forced a `FullResync` on every remote notes edit. The
  marker write (a `Map` diff on the item with key `notes`) already mapped
  to the same key set.
- `import_json` wrote the `notes` register directly rather than through
  `edit_item_notes`; it now creates the text container and inserts.
- Events: `ItemNotesChanged { id, notes }` keeps carrying the full string
  (the store, search index, and CLI want plain text); `ItemTextChanged`
  is unchanged. Add `ItemNotesDelta { id, delta }` later in Phase 2; not needed for
  Phase 1.
- Hash (`hash_str(&i.notes)`), JSON export / import, duplicate-list copy:
  unchanged because they go through `ItemView` strings and the edit
  functions.
- Undo: core's doc-wide `UndoManager` will start recording each notes
  commit as a workspace undo step. Phase 1 leaves that as-is (it already
  records whole-string sets). Phase 2 revisits (see Undo below).
- Schema: there is no version constant in code; the bump is the `doc.rs`
  module comment plus the `data-model.md` "v3 → v4" paragraph. The JSON
  export shape is unchanged (strings, `version: 1`), so v3 exports import
  into v4 as-is. A v3 doc opened by v4 reads notes as empty; the first
  notes write to such an item fails because `ensure_mergeable_text`
  refuses a key holding a plain value.
- An unchanged write (`update` to the current string) writes no ops and
  emits no event.

Tests (extend the existing multi-peer tests in `core/src/doc.rs`):

1. Two peers write notes to an item that had none, offline, then sync:
   both texts present, one container (the mergeable-id guarantee).
2. Two peers edit different parts of existing notes: both edits present.
3. Same-region concurrent edits: character-level merge, nothing dropped.
4. Remote notes edit produces `ItemNotesChanged` for that item only, no
   `FullResync` (asserts the classifier arm).
5. Clear notes on one peer while another appends: the appended text
   survives (content delete and a concurrent insert merge as ordinary text
   ops; no marker race because the key is never deleted). Also assert that
   clear-then-type on one peer yields only the typed text, which is the
   resurface bug the key-delete design would have had.
6. Export v3 JSON, import into v4, hash equal.

Estimate: 1 day including the cutover.

### Phase 1 measurements (2026-09-08)

`bun run perf scale` (release, `core/examples/perf.rs`), 10k items with two
paragraphs of notes each (4.3 MiB of notes text):

| Shape | Snapshot | Import | Boot walk (location only) | `to_string` every notes |
|---|---|---|---|---|
| v3 string register via `Doc` | 6.6 MiB | 40 ms (incl. index rebuild) | 20 ms | 34 ms (`all_items`) |
| v4 `LoroText`, one commit per item | 6.6 MiB | 3.5 ms | 13 ms | 38 ms |
| v4 `LoroText`, 5 commits per paragraph | 6.1 MiB | 2.8 ms | 14 ms | 39 ms |

A text container per noted item costs nothing at boot and nothing in
snapshot size. The typing-burst history only matters for uncompacted op
replay (Phase 2's commit policy), not for snapshots.

Done 2026-09-08. Tests: `notes_*`, `remote_notes_edit_translates_to_one_surgical_event`,
`v3_string_register_notes_read_as_empty_and_hash_stable`,
`v3_json_export_imports_into_v4_with_notes` in `core/src/doc.rs`.

## 3. Editor bindings: what exists and why none plug in

| Binding | Version (date) | Model | Verdict for Monoplan |
|---|---|---|---|
| `loro-prosemirror` (official) | 0.4.4 (2026-08-22) | PM node tree as `LoroMap{nodeName, attributes, children: LoroList}` with `LoroText` leaves; sync, undo, ephemeral cursors | Needs a JS `LoroDoc`. Tree model also means Rust could not render notes to text without reimplementing the PM shape. |
| `loro-codemirror` (official) | 0.3.3 (2025-10-07, one commit since) | single `LoroText`, undo, cursors | Needs a JS `LoroDoc`. Its bridge is ~150 lines and is the template for ours. |
| Quill | in-repo example only (Quill 1.3.7) | `LoroText` delta, strings only | No package, but Loro's `TextDelta` is the Quill delta format by design. |
| `loro-slate`, `lexical-loro`, ProseKit `defineLoro` | community / wrappers | trees over loro-prosemirror or their own | Same JS-doc requirement. |
| Tiptap 3 | Yjs only officially | | loro-prosemirror can be registered as raw PM plugins (cooee did this), still JS-doc bound. |

The blocking fact: **Monoplan's `LoroDoc` is inside `monoplan-core-web`
(Rust wasm).** Every binding constructs against `loro-crdt`'s JS `LoroDoc`.
Shipping `loro-crdt` too would add a second Loro runtime (1.05 MB gzipped
wasm plus glue) and require mirroring the notes container between two
runtimes on every keystroke. Rejected.

What we build instead: a **delta bridge** on the wasm API.

```
// core (Doc) and wasm (core/web/src/lib.rs), built 2026-09-08
subscribeNotes(itemId): string        // current plain text; starts the delta stream for this item
unsubscribeNotes(itemId)              // editor closed
applyNotesDelta(itemId, deltaJson)    // UTF-16 in, validated whole, converted, one commit, origin "notes:<itemId>"
// event
itemNotesDelta { id, delta }          // remote / other-tab / undo changes as a UTF-16 delta, subscribed items only
```

Delta shape is Quill's, plain text only for now: a JSON array of
`{"retain": n}` / `{"insert": "s"}` / `{"delete": n}`
(`monoplan_core::NotesDeltaOp`, `serde(untagged)`).

Rust side: `ensure_mergeable_text`, validate the whole delta against the
current text (bounds, and no position inside a surrogate pair; a bad
delta is rejected with `Invalid` and nothing is written), then walk it
with a UTF-16 cursor calling `insert_utf16` / `delete_utf16`, commit with
origin `notes:<itemId>`. Emits the whole-string `ItemNotesChanged` (store,
search) and never echoes an `ItemNotesDelta` for a local apply. Event
side: the diff classifier's text arm now carries the Loro delta
(`CapturedDiff::ItemText`); at emission, a subscribed item's delta is
converted scalars-to-UTF-16 against its shadow string (see Index units),
the shadow advances, and `ItemNotesDelta` follows the `ItemNotesChanged`.
If a delta does not fit the shadow, or the change was wholesale (a
container re-set, a view-diff path), a full replace delta is emitted
instead. `FullResync` refreshes every shadow; the editor re-subscribes
on it. Subscription is explicit (`subscribe_notes` returns the text and
seeds the shadow), so unsubscribed items cost nothing.

Web adaptor (`TaskDialog.tsx`, `notesDelta.ts`): the editor keeps
`synced` (the core's text) beside the DOM; every `input` event diffs
them (common prefix / suffix in UTF-16 units, never splitting a
surrogate pair) and sends the delta. An inbound delta is applied to
`synced`, the DOM is re-rendered through the linkifier, and the caret is
moved through the delta (`transformOffset`). IME: nothing is sent
mid-composition; a remote delta that lands during one is applied to
`synced` and the composed text is re-placed on top at `compositionend`
(its position shifted through the inbound delta, its deletion kept only
if the remote edit did not touch that range). Closing, target switch,
blur, `visibilitychange` (hidden) and `pagehide` flush. The dialog renders
**two** notes editors (the new-item capture form and the existing-item
edit form); both bind the same handlers. The first Phase 2 cut wired only
the capture form, so typing in an existing item sent nothing until Enter
or close. Fixed 2026-09-08.

Commit policy, as built: `applyNotesDelta` commits on every editor
change, but that is not one op blob per keystroke. Loro merges
consecutive commits from one peer into a single `Change` when nothing
remote interleaves and the timestamps are within its merge interval
(origin is not part of the change, so the `notes:` tag does not split
them; verified 2026-09-08: 20 origin-tagged commits, `len_changes() ==
1`). What decides the blob count is the capture (`captureLocalOps`, one
WAL row and one push per `flush`), so the coalescing lives in the store:
`applyNotesDelta` skips the store's `mutate` / `flush` path (no workspace
undo step, no capture) and arms a 300 ms idle timer; `flushNotes` runs
the capture now on blur / close / target switch / page hide. A typing
burst is therefore one op blob holding one merged change. Compaction for
the offline-by-default case (fold with retained outbox rows) remains a
prerequisite for long offline editing sessions.

Undo consequence: with notes commits excluded from the workspace
`UndoManager` by origin prefix (see Undo under Rich text; the prefix hook
already exists in core for `remote`), the core never records a notes
step. The editor's own history owns notes undo while it is focused. Once
focus leaves, the web store steps in (built 2026-09-23): `subscribeNotes`
opens an editing session, and `endNotesSession` (editor blur, target
switch, unsubscribe) records the session's net change as one entry on
the store's undo stack beside the core step counts. Undo / redo of that
entry re-applies the inverse delta through `applyNotesDelta` (still the
excluded origin, so no core step is registered) and notifies subscribed
editors like any inbound delta. A session whose text ends where it began
(fully undone natively while focused) records nothing, so the two
histories never hold the same edit. Peer edits since the step shift or
survive around the region; a delta the core rejects drops the step.

Phase 2 deliverable: this bridge plus the current plain contenteditable
dialog switched from full-string writes to deltas, so a live remote edit
under the caret no longer replaces the whole field. Estimate: 1.5 days.
**Done 2026-09-08.** Tests: `apply_notes_delta_*`, `notes_delta_*`,
`remote_notes_edit_streams_a_utf16_delta_to_subscribers`,
`remote_delta_converts_against_the_locally_advanced_shadow`,
`undo_of_whole_string_notes_write_streams_a_delta`,
`utf16_conversion_and_compose_helpers` in `core/src/doc.rs`;
`js/web/test/notesDelta.test.ts` for the editor diff / caret transform.
`edit_item_notes` (whole string, default origin, one undo step) is gone
(2026-09-23): `apply_notes_delta` is the only notes write. Every caller,
including the CLI's welcome seed, the web capture form, and duplicate,
sends the diff against the current text, so notes never enter the
core's UndoManager on their own (`import_json` still fills the text
container inside its own single import commit). The web store's
`setItemNotes` is the whole-value convenience over the delta path; its
undo bookkeeping is described under "Undo consequence" above. A delta
that changes nothing writes no op and emits no event. Not verified in a
browser (no automation here): the IME re-placement path and caret
restoration are covered by reading and the unit tests only.

## 4. Rich text (Phase 3)

### Model: flat rich text in one `LoroText`

Loro's text is a flat sequence with marks; it has **no embeds** (Rust
`TextDelta::Insert { insert: String, .. }`, and `applyDelta` with an object
insert throws). The two official ways to get structure are:

1. A node tree (`LoroList` / `LoroMap` / `LoroText` leaves), which is what
   loro-prosemirror and loro-slate do. Full block structure, but a tree the
   Rust side cannot render without a PM-shaped walker, several containers
   per note, and a much larger bridge.
2. **Quill-style flat rich text**: inline marks (`bold`, `italic`, `code`,
   `strike`, `link`) on ranges; block formats (`header`, `list`,
   `code-block`, `blockquote`) as attributes on the `\n` that terminates
   the line; images as a placeholder character with an attribute.

Option 2 is chosen. It keeps notes in one container, `to_string()` is the
plain-text projection for search, CLI, export, and hashing (with `U+FFFC`
rendered as `[image]`), and the wasm bridge from Phase 2 already carries
attributes.

Style config (`LoroDoc::config_text_style`, set once at doc open, same on
every client): `bold` / `italic` / `strike` / `code` expand `after`;
`link` expand `none`; line attributes (`header`, `list`, `blockquote`,
`code-block`) expand `none`; `image` expand `none`. Loro requires a key to
always use one expand type, so the table lives in one place in core.

### Editor

Quill 2 is the natural fit: its native data model is the same delta, line
formats live on `\n`, images are embeds, and its history module handles
remote transforms with `userOnly: true`. The adaptor translates
`{ insert: { image: src } }` to `insert: "￼"` with
`attributes: { image: <ref> }` and back. Quill's default themes are replaced
with Monoplan's own toolbar and styles (headless usage is supported).

CodeMirror 6 with Markdown is the fallback if Quill's contenteditable
handling fights the dialog (mobile caret, IME, the existing
`locateOffsetInLinkified` model). It is plain `LoroText` with no marks and
a Markdown preview, images as `![](ref)`. Cheaper, less WYSIWYG. Decide by
prototype in Phase 3, not now.

### Undo

Core's doc-wide `UndoManager` is the workspace undo (moves, lifecycle,
edits). Typing in a rich editor should not push keystroke groups into it,
and the editor's own undo needs to survive remote inserts. Rule: notes
commits carry origin `notes:<itemId>`; the workspace `UndoManager` gets
`add_exclude_origin_prefix("notes:")`; the editor's history module owns
text undo while the editor is open. This is the same scoping problem
tracked upstream as loro-dev/loro#981 (per-container undo); the origin
prefix is the workaround Loro itself suggests.

### Native clients (iOS, Android)

Researched 2026-09-08: no iOS or Android editor binding for `LoroText`
exists, official or community, and Lexical iOS has no collaboration
support at all, so native clients bind to the same delta bridge over
`monoplan-ffi` (uniffi) that the web uses over wasm. On iOS that is a
`UITextView` (TextKit 2, wrapped for SwiftUI) with `NSTextStorageDelegate`
producing UTF-16 deltas and `itemNotesDelta` applied to the text storage;
marks map to font traits and attributes, line formats to
`NSParagraphStyle` + `NSTextList`, and images to `NSTextAttachment`,
whose placeholder is the same `U+FFFC`. Android is the same shape with
`EditText` spans and `TextWatcher`. SwiftUI `TextEditor` is adequate only
for the plain-text phases (whole-string `update`, as Automerge's
MeetingNotes does). Cross-client equivalence comes from a shared contract,
`spec/notes-format.md` (mark keys, expand table, line attributes, image
attribute shape, plain projection) plus a delta fixture set every adaptor
must pass, not from a shared editor. Native undo routes to the
notes-scoped core `UndoManager`. Estimate: about 2 days per platform on
top of Phase 3.

### What carries over from cooee

Cooee (`danielgormly/cooee`, not on disk any more; `../cooee` is gone) is
one `LoroDoc` per post with `loro-prosemirror` 0.4.3 (`LoroSyncPlugin` +
`LoroUndoPlugin`, no cursor plugin), a PM schema with `image` as a block
node holding a URL, S3 uploads referenced by id, a `contentJson` string
mirror so the server can read the doc, and snapshot-swap (`TAG_REPLACE`)
room initialisation.

Transfers: images by reference only; the mark expand table derived from
"inclusive" semantics; paste sanitisation as a whitelist DOM walk; the
warning that a whole-doc `UndoManager` swallows editor undo.

Does not transfer: per-post docs and the whole replace / epoch machinery
(single-doc here); server-side reads and the JSON mirror (E2EE server);
rebuilding the editor on snapshot import; loro-prosemirror itself (JS doc).

## 5. Images (Phase 4, own spec)

Bytes never enter the CRDT: one photo would be a megabyte op blob in every
snapshot and every device's WAL. Instead:

- New server surface, `spec/attachments.md`: `PUT /attachments/<id>` and
  `GET /attachments/<id>` per account, opaque encrypted blobs, size cap
  (proposal 8 MiB), sqlite table `attachments(account_id, id, bytes,
  created_at)`. Same dumbness as ops: the server cannot read them.
- Client: downscale to at most 2048 px on the long edge, encode WebP,
  encrypt with the account DEK (AES-GCM, fresh nonce, per
  `encryption.md`), upload, then insert `U+FFFC` with
  `image: { id, mime, w, h }`. Cache decrypted bytes in IndexedDB (web) or
  the sqlite store (CLI, if it ever renders).
- Garbage: the server cannot count references. Pre-release rule: hard
  delete of an item deletes the attachments its notes reference (client
  issues the deletes). A periodic client-side sweep is future work; leaks
  are bounded by the size cap.
- Sync of attachments is separate from op sync and needs no protocol
  change; a note referencing an attachment the device has not fetched
  shows a placeholder until `GET` succeeds.
- Snapshots do not include attachments. Durability is the attachments
  table.

Estimate: 3 days including the server spec, tests, and the web upload path.

## 6. Order of work

| Phase | What | Days |
|---|---|---|
| 0 | Raise the `loro` floor (1.13, then 1.16.0), build wasm, run tests. **Done 2026-09-08.** | 0.25 |
| 1 | `notes` as mergeable `LoroText` (`text` stays a register), `update` diffing, content-clear (no key delete), classifier arm, schema v4 cutover, merge tests. **Done 2026-09-08.** | 1 |
| 2 | Delta bridge (`_utf16` inbound, shadow-string outbound), coalesced capture, dialog writes deltas, live remote edits under caret. **Done 2026-09-08.** | 1.5 |
| 3 | Rich text: style config, Quill 2 adaptor (or CodeMirror fallback), toolbar, paste whitelist, plain projection with `[image]` | 3 |
| 4 | Attachments spec + server + client upload, image insert | 3 |

Phase 1 is worth doing alone: it removes the one CRDT failure users notice
(the dropped notes edit between phone and laptop when both were offline)
and is a prerequisite for sharing. It does not fix an open dialog being
overwritten by a live remote edit; that is Phase 2. Phases 3 and 4 are
product decisions and can wait.

## Open questions

- ~~Should `text` (the title) ever carry marks?~~ Settled: `text` stays a
  plain string register, so the row renderer never parses attributes.
- Line-format vocabulary: headers and lists yes; tables, embeds other than
  images, and nested lists no. Revisit if notes grow.
- Whether the CLI should print marks (Markdown-ish) or plain text.
  Plan: plain text from `to_string()` with `[image]`.
- Attachment retention after item bin vs hard delete: bin keeps, hard
  delete removes. Restore therefore never loses an image.
