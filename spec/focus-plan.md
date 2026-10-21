# Focus + Inbox rename — implementation plan

Status: **Phase 0–4 DONE; Phase 5 verification DONE — only the live-doc cutover
remains (Daniel triggers the wipe).** See "Progress & handoff" at the bottom for
exactly what's built, what deviated from this plan, and where a fresh context should
pick up. This doc is the handoff for that context.
Two features:

1. **Rename "Queue" → "Inbox"** (label only; `main` id and settings keys unchanged).
2. **Add "Focus"** — a reserved, single-tier, curated *list-by-reference*: it points
   at items that live in other lists rather than owning them.

Product decisions already settled (do not relitigate — see below for the *why*):

- **Focus is a single tier.** No "Next"/"Someday"/on-deck sub-tiers. The tool asks
  the user for discipline instead of inventing parking spots for indecision.
- **No scheduling / no dates for Focus.** "Do it first thing tomorrow" is a
  *sequencing* statement, not a time statement: you drop the item into Focus now,
  ordered at the top, and it's there when you sit down. Deferring to a date is a
  separate, deliberately un-prominent future feature (hard-landscape items only),
  out of scope here.
- **Focus is a lens, not a home.** An item in Focus still lives in its source list.
  Removing from Focus removes the *reference*, never the item. Acting on an item in
  Focus (mark Live/Done) mutates the item everywhere.
- **Focus self-cleans via lifecycle.** A referenced item that goes Done or Binned
  evaporates from the Focus view. Focus shows only Open (Backlog + Live) references.
- **Reference-first for a reason:** today Focus references items in the local doc;
  the sharing feature (`spec/sharing-plan.md`) will need Focus to reference items in
  *other* docs. The reference encoding must be forward-compatible with cross-doc refs
  from day one so no migration is needed when sharing lands.

---

## Part A — Rename `main`/`home`/Queue → Inbox (id + label)

**Decided: full flip, including the stored reserved id** (`main` → `inbox`). This is a
**stored-data change, not additive** — the reserved literal appears in every item's
`location` register and in the order-container name. It rides a **one-time export → wipe
→ import cutover** on the live production doc, the same procedure as the documented
v1→v2 break (`spec/data-model.md` "Schema versioning & compatibility"). The container
*shapes* don't change (only the reserved literal and the `order/*` name), so no
schema-version renumber is needed — keep it "v2" and do the cutover.

### A.1 Cutover (the one operational cost)

1. Export JSON on the current build.
2. Wipe the account + local databases (client IDB, server op log/sqlite).
3. Import on the new build.

**The importer must alias the legacy reserved id `main` → `inbox`** — the export JSON
carries `list_id: "main"` for reserved-list items, and the new build reserves `inbox`.
Either add a one-shot alias in the JSON import path (`main` ⇒ `inbox` for the reserved
list only), or `sed` the export file before import. Pick the in-code alias; it's a few
lines and keeps the cutover a pure data round-trip. (Confirm this is the *only* place
`main` is stored verbatim in exported JSON — grep the export path.)

### A.2 Stored-id edit sites (the CRDT-facing ones)

- `core/src/doc.rs:59` — `LIST_MAIN: &str = "main"` → rename const to `LIST_INBOX` and
  set value `"inbox"`. Re-export in `core/src/lib.rs:12`.
- `core/src/doc.rs:60` — `LIST_MAIN_NAME: &str = "Queue"` → `INBOX_NAME` = `"Inbox"`.
- Order container name `order/main` → `order/inbox` (wherever the container name is
  built from `LIST_MAIN`).
- `Location` / `OrderEntry` grammar reserves the literal — flip `main` → `inbox`
  (`spec/data-model.md:98-99`, and the parse/guard code in `core/src/doc.rs` ~305-357).
- Settings key `KEY_MAIN_NAME`/`"main_name"` (`core/src/doc.rs:106`) → `"inbox_name"`;
  mutation `set_main_name` → `set_inbox_name` (`doc.rs:1867`, WASM `core/web/src/lib.rs:246,
  1400`, store `js/web/src/sync/store.ts:209`).
- All reserved-id guards/defaults keyed on `"main"` in `core/src/doc.rs`
  (rename/move/delete refusals, synthesized `ListView`, import id-map, fallback defaults
  — Explore flagged `1867, 1893-1972, 2176-2180, 2250-2256, 2285, 2418, 3383, 3463-3538,
  3594`). Grep `LIST_MAIN` and the literal `"main"` and flip each.
- CLI `--list` default (`cli/src/commands/items.rs:24, 72`) now resolves to `inbox`.
- Web reserved-id literals `{ kind: "list", id: "main" }`: `js/web/src/prefs.ts`,
  `runtime.ts:84-86`, `nav.tsx:227-228, 402`, `Workspace.tsx:890`.

### A.3 Label / i18n / naming (the cosmetic ones)

- `js/web/src/i18n.tsx` — rename the key `home` → `inbox`; English value `"Queue"` →
  `"Inbox"` (`:382`), **Spanish** `"Cola"` → `"Bandeja de entrada"` (`:230`); update key
  declarations (`:62, :229, :381`) and every `t("nav.home")` call site.
- `cli/tests/sync_smoke.rs:257` — assertion `"Queue"` → `"Inbox"`.
- Doc-comment sweep: "Queue"/"non-Queue" → "Inbox"/"non-Inbox" across `core/src/doc.rs`,
  `core/src/events.rs`, `core/web/src/lib.rs`, `core/ffi/src/lib.rs`, `js/web/src/nav.tsx`,
  `js/web/src/sync/store.ts`.
- `spec/data-model.md` — "rendered as Queue" (~261), "non-Queue"/"Queue's count"
  phrasings (~255, 275-276, 294-295), and the reserved-literal note (98-99).

**Sequencing:** do Part A's stored-id rename in **Phase 1** (it's core Rust + a cutover),
*before* the Focus core work, so Focus is built against `inbox` from the start. Land the
cutover on the live doc at a clean checkpoint.

---

## Part B — Focus

### B.1 Why Focus can't reuse the list/order machinery

`spec/data-model.md`: an item's `location` register is `"<list_id>:<placement_id>"` —
a single atomic value naming exactly **one** list. Order containers (`order/<list-id>`)
only become *visible* for an item when `item.location.list_id == L`. That is
single-membership by construction. Focus is a *second* appearance of an item without
moving it out of its home list, so it cannot ride on `location` or the order-container
visibility rule. Focus needs its own container of references.

(Confirmed greenfield: there is no existing reference/alias/pointer concept in the doc
model to extend — `location` is the only membership mechanism.)

### B.2 New container

Add one reserved singleton container, analogous to `order/main`:

- `doc.get_movable_list("focus")` — a `LoroMovableList` of **encoded scalar FocusRef
  strings only** (never child containers — same discipline as order containers). The
  MovableList's own order *is* the Focus order; reorder via `MovableList::mov`.
- Not a `ListMeta` row (like `main`). Reserved, non-deletable, non-renamable.
- **Additive** to the schema-v2 layout — it reinterprets no existing container, and
  Loro roots are typed by (name, type), so a focus-unaware client simply never projects
  it. Recommend treating this as additive within v2 (no version bump); confirm with
  Daniel. Document it in `spec/data-model.md`'s layout section. The sqlite migration
  (`001_init.sql`) is unaffected — it stores opaque blobs.

### B.3 FocusRef grammar (forward-compatible for cross-doc)

```
FocusRef = "<item_id>"              # now: local doc, bare uuid-v7 hex
         = "<doc_id>:<item_id>"     # future: cross-doc (sharing)
```

Same `:`-on-first-colon convention as `Location`/`OrderEntry`; uuid-hex components mean
`:` never collides. **Parser accepts both forms now** (no colon ⇒ local doc); **emitter
writes only the bare form** until sharing lands. This is the whole point of designing it
now — cross-doc refs slot in with zero migration. A ref whose `doc_id` is not the local
doc is *unresolvable today*; projection skips it (later: renders a placeholder).

### B.4 Projection & visibility

`focus_view()` → ordered `Vec<ItemView>`:

```
visible(ref in focus) :=
     ref resolves to the local doc
  && items[ref.item_id] exists
  && items[ref.item_id] is Open           # done_at == null && binned_at == null
  && no earlier visible ref has the same item_id     # dedup, first wins
```

Refs to missing / done / binned / foreign-doc items are **harmless garbage** — filtered
out by projection, GC'd by `reconcile()`. This mirrors the existing "stale order entries
are harmless" invariant. **Reads never mutate** (no self-repair on projection).

### B.5 Lifecycle interplay — the key semantic decision

> **SUPERSEDED (2026-07-12).** After investigation, Daniel chose **auto-remove the
> focus ref on Done** — the opposite of the recommendation below. Rationale and the
> final contract live in `spec/focus.md` "Lifecycle interplay" and Open-decision #1
> above. The text below is kept for the reasoning trail only.

**Recommended: keep the ref, filter by Open.** `set_item_lifecycle` stays a single
map-write and **never touches the focus container** — exactly the discipline the board
adopted for order containers (`data-model.md` "Done / binned items stay in the order
container"). Consequences:

- Done/Binned ⇒ item drops out of Focus view automatically (Open filter). Ref survives
  as garbage.
- **Un-done ⇒ the item reappears in Focus in its former position** (ref survived). This
  is the trade of keep-and-filter. It's consistent with restore-reveals-former-position
  elsewhere, and arguably nice ("this came back"). **Flag for Daniel to confirm.**
- Hard delete ⇒ ref becomes garbage (item lookup fails), GC'd by reconcile.
- Delete-list bins its items ⇒ they drop from Focus automatically. No special handling.

Rejected alternative: auto-remove the focus ref on Done. It couples the most common
mutation to a second container (two-container write), breaks the single-write invariant,
and makes un-done non-restorative. Same reasoning the board used to reject removing order
entries on lifecycle flips.

### B.6 Focus is a flat list, not a board

Focus renders as a single ordered flat list of Open items. You can still flip an item
Live/Done from within it, but there are **no lanes** (no Backlog/Live/Done split, no
board lens). Keep it deliberately minimal — that's the product thesis.

> **Dropped 2026-10-21:** no longer a settled decision. A board lens over
> Focus is open for consideration (`spec/focus.md` no longer lists it).

### B.7 Core API surface (`core/src/doc.rs`)

New mutations (each **one Loro commit**):

- `add_to_focus(item_id, index)` — insert a FocusRef at `index` (default append). If the
  item already has a *visible* ref, **no-op** (recommend; don't move-to-top — confirm).
- `remove_from_focus(item_id)` — remove the item's ref(s) from the focus container. Item
  untouched.
- `move_in_focus(item_id, index)` — reorder within Focus (`MovableList::mov`).

Reads:

- `focus_view()` / `focus_refs()` — the projection above.

Maintenance & integrity:

- Extend `reconcile()` to prune focus refs that are missing / done / binned / foreign,
  and to dedup. Idempotent; no-op when clean.
- **`doc_fingerprint` must hash the focus order** — it's logical state (fixes the curated
  order), exactly as it hashes each list's resolved order.

Constants: add `FOCUS_CONTAINER` (`"focus"`) alongside the existing container-name
constants near `LIST_MAIN_NAME`.

### B.8 Events (`core/src/events.rs`)

Add a focus-changed event variant so clients re-project on focus mutations. Check how
per-container diffs are currently surfaced and mirror that. Note: a lifecycle change that
makes a focused item done/binned already emits an item event — the web store must
re-derive `focus_view` on item events too (visibility depends on item lifecycle), not
only on focus-container events.

### B.9 WASM bindings (`core/web/src/lib.rs`)

Expose `add_to_focus`, `remove_from_focus`, `move_in_focus`, and the focus projection
getter. **Note there are two handle structs** in this file (binding surfaces at ~L107 and
~L1236) — add the methods to **both**, mirroring how `move_item` / `set_item_lifecycle`
appear twice. `bun run build:wasm` from the workspace root.

### B.10 FFI + CLI (`core/ffi/src/lib.rs`, `cli/src/`)

Multi-device proof spans CLI ↔ web, so CLI needs parity. Add FFI wrappers and CLI
commands per `spec/cli.md` conventions, e.g.:

- `monoplan focus` — list the Focus view
- `monoplan focus add <item>` / `monoplan focus rm <item>` / `monoplan focus mv <item> <pos>`

### B.11 Web UI (`js/web`)

The clean extension point: the view model is a discriminated union
`ViewKey = { kind: "list"; id } | { kind: "done" } | { kind: "bin" }` at
**`js/web/src/prefs.ts:19-22`**. Add `{ kind: "focus" }` there; it's persisted to prefs
and owned by `js/web/src/sync/runtime.ts:77-90`, dispatched in `Workspace.tsx` via
`view().kind` branches (e.g. `Workspace.tsx:194-200`, `:406-415`, `:1009-1016`).

- `i18n.tsx` — add a `focus` label (en **and es**). (The `home` → `inbox` key rename is
  Part A.)
- `nav.tsx` — a reserved **Focus** entry alongside the existing static Done (`nav.tsx:436`)
  and Bin (`nav.tsx:449`) entries; the Home/Inbox row is at `nav.tsx:400-410`. Put Focus
  near the top, above user-created lists. Count badge = number of visible refs. (Note:
  the reserved list synthesizes its `ListView` and can't carry an icon — Focus is a
  static nav entry like Done/Bin, not a `ListMeta` row, so give it a fixed icon.) **Make Focus feel finite:**
  a soft signal past ~7–10 items (color shift / subtle "getting big" — not a hard cap).
  Threshold + exact treatment is a UI decision; keep it gentle.
- **Focus view component** — flat ordered list of `focus_view()`; drag-to-reorder
  (reuse the existing DnD infra in `js/web/src/dnd/`); per-row actions: toggle Live,
  mark Done, and **remove-from-focus as a single cheap gesture** (× / swipe). Removing
  must be as frictionless as adding — that's what keeps Focus lean.
- **Add-to-focus affordance** on any item row (`Row.tsx` / item menu / `TaskDialog.tsx`)
  — a pin/star toggle whose state reflects whether the item currently has a visible focus
  ref. Toggling calls `add_to_focus` / `remove_from_focus`.
- `sync/store.ts` — expose the focus projection + mutations; subscribe to focus events
  and re-derive `focus_view` on item events (per B.8).

### B.12 Sync protocol

**No change.** Focus mutations are ordinary Loro ops in the same doc — opaque blobs to
the server. No wire-version bump, no server work.

---

## Part C — Specs to write/update

- **New `spec/focus.md`** (mirror `spec/board.md`'s structure): the Focus lens —
  reference model, FocusRef grammar, projection/visibility rule, lifecycle interplay,
  flat-not-board decision, and the "single tier, no scheduling" product rationale.
- `spec/data-model.md` — add the `focus` container to the layout; FocusRef grammar; focus
  mutations in the mutation-contracts + rust-API sections; fingerprint note; the Inbox
  rename.
- `spec/cli.md` — the `focus` commands.
- Cross-link from `spec/sharing-plan.md` (cross-doc FocusRef is the forward hook).

---

## Phasing (checkpoint with Daniel between phases)

- ✅ **Phase 0 — Specs. DONE.** `spec/focus.md` written; `data-model.md`, `cli.md`,
  `board.md`, `sharing-plan.md`, and the `AGENTS.md`/`CLAUDE.md` spec table updated;
  Inbox rename swept across specs.
- ✅ **Phase 1 — Core (Rust). DONE, all tests green.** `main`→`inbox` rename landed
  (1a) and Focus core landed (1b). Details + deviations in "Progress & handoff". The
  **live-doc cutover was deliberately NOT run** — moved to after Phases 3–4 (see
  handoff note 5).
- ✅ **Phase 2 — WASM. DONE.** Added `addToFocus` / `removeFromFocus` / `moveInFocus` /
  `focusRefIds` / `focusViewJson` to **both** handle structs in `core/web/src/lib.rs`
  (`Doc` and `SyncEngine`). Added the `focusChanged` variant to `AppEventJs::from`
  (no payload — fixes the non-exhaustive match Phase 1 left broken) + its doc-comment
  entry. Renamed the JS-facing inbox surface: `setMainName`→`setInboxName` (both structs),
  the `AppEventJs` getter `mainName`→`inboxName`, and the settings-JSON key
  `"mainName"`→`"inboxName"` in `settings_to_json`. `bun run build:wasm` green; `.d.ts`
  regenerated. **Watch-out for Phase 3:** the web store still reads `settings.mainName` /
  `settingsChanged.mainName` and calls `setMainName` — those JS call sites now break until
  Phase 3 flips them to `inboxName` / `setInboxName` (already on the Phase 3 list).
  Note: `cargo check -p monoplan-core-web` fails off-target (E0308, pre-existing wasm-only
  crate) — build it only via `bun run build:wasm`.
- ✅ **Phase 3 — Web. DONE, all JS tests green.** i18n, nav Focus entry, Focus lens
  render, add-to-focus toggles, store/event wiring, and the `main`→`inbox` literal sweep
  all landed. Details + deviations in "Progress & handoff" (Phase 3 section). Notable
  choice: **no new Focus view component** — the Focus lens reuses the existing list-view
  `Dnd` render path (branch on `view().kind === "focus"`) rather than a `Board`-style
  standalone component, so selection / expansion / open-dialog / global shortcuts come for
  free. In-browser verification was done by Daniel (single context); the **two-tab
  convergence + CLI↔web fingerprint parity checks move to Phase 5**, since CLI focus
  parity doesn't exist until Phase 4.
- ✅ **Phase 4 — CLI/FFI. DONE.** `monoplan focus` / `focus add` / `rm` / `mv` commands
  (`cli/src/commands/focus.rs`, wired in `commands/mod.rs`); FFI wrappers
  (`add_to_focus` / `remove_from_focus` / `move_in_focus` / `focus_view` on
  `MonoplanStore`). CLI↔web convergence + fingerprint-parity integration test
  (`focus_curation_converges_across_devices` in `sync_smoke.rs`). Details in "Progress
  & handoff" (Phase 4 section).
- **Phase 5 — Verify (DONE) + cutover (PENDING Daniel).** ✅ Multi-device convergence,
  fingerprint parity across CLI devices, self-clean-on-done, un-done-does-not-reappear —
  all covered by the automated integration test **and** a live server-backed CLI drive
  (two devices, real WS, bidirectional). ⏳ **Remaining:** the live-doc export → wipe →
  import cutover (handoff note 5) — dry-run on a scratch account first; **Daniel triggers
  the wipe**, not the agent.

---

## Progress & handoff (as of 2026-07-12)

Phases 1–3 are on `main`'s working tree (commit state per git). Full `cargo test` green;
`cargo fmt` clean; no new clippy hits (repo lint was already red pre-existing). Web +
core **JS** suites green (`bun test` in `js/web` and `js/core`) and `tsc --noEmit` clean
in `js/web`.

**Phase 1a — `main` → `inbox` rename (done).**
- `core/src/doc.rs`: `LIST_MAIN`→`LIST_INBOX` (`"main"`→`"inbox"`), `LIST_MAIN_NAME`→
  `INBOX_NAME` (`"Queue"`→`"Inbox"`), `KEY_MAIN_NAME`→`KEY_INBOX_NAME`
  (`"main_name"`→`"inbox_name"`), `set_main_name`→`set_inbox_name`; re-exported in
  `lib.rs`. `order/inbox` follows from the const. Doc-comment sweep done.
- **Import alias:** added `pub const LEGACY_LIST_MAIN = "main"`; the JSON importer maps
  legacy `main` ⇒ `inbox` (id-map seed + the builtin/legacy list guard). This is the
  **only** place `"main"` is still referenced — the cutover bridge.
- WASM getter `main_name()`→`inbox_name()` renamed in `core/web` (Rust side only; the JS
  call sites are Phase 3). CLI `--list` default already resolves to `inbox`.

**Phase 1b — Focus core (done).** In `core/src/doc.rs` unless noted:
- `FOCUS_CONTAINER="focus"`; `FocusRef` (`encode`/`parse`, both forms, `is_local`);
  `FocusScan` + `scan_focus()` (classify raw refs → visible/dead) + `prune_focus_refs()`
  (delete matching + dead in one uncommitted batch).
- Public: `add_to_focus(item,index)` (no-op if already visible or not Open; `ItemNotFound`
  on unknown; folds sweep), `remove_from_focus(item)`, `move_in_focus(item,index)`,
  `focus_refs()`, `focus_view()`. Index addresses the **visible** focus order (raw==visible
  after the folded sweep).
- **Auto-remove-on-Done wired into THREE paths** (the gotcha): `set_item_done`,
  `set_items_timestamp` (key==`done_at`, on), **and** `set_items_lifecycle(Done)`. The
  board path is `set_item_lifecycle`; CLI/other use `set_item_done`/`set_items_done`. Any
  future "done" path must also prune. Binned deliberately does **not** prune.
- `AppEvent::FocusChanged` (no payload) in `core/src/events.rs`; emitted on every
  focus-mutating local commit and on Done-that-clears-a-ref; remote frames classify via
  new `CapturedDiff::Focus` → `FocusChanged` (surgical, no FullResync).
- `reconcile()` extended to prune focus dead refs + dedup. `fingerprint()` hashes the raw
  focus container order.
- 13 focus unit tests in the `doc::tests` module (all green), incl. the revised
  **done-auto-removes** / **un-done-does-not-reappear** cases (NOT the old
  "un-done-reappears").

**Phase 3 — Web (done).**
- **Inbox rename swept across `js/web`:** every `"main"` reserved-id literal → `"inbox"`
  (prefs/runtime/nav/Workspace/FindPalette/TaskDialog); `SettingsView.mainName`→`inboxName`
  + all reads; `setMainName`→`setInboxName`; i18n key `home`→`inbox` (en `Inbox`, es
  `Bandeja de entrada`). `grep '"main"'` in `js/web/src` now only hits the `<main>` tag.
- **JS test fallout:** `js/core/test` + `js/web/test` hardcoded the reserved id as `"main"`
  (const `LIST_MAIN`, inline literals, a `listOpen.main` assertion). Flipped their *values*
  to `"inbox"` (kept the `LIST_MAIN` const name). These broke the moment Phase 2 rebuilt the
  wasm; not enumerated in the original Phase 3 list.
- **Store (`sync/store.ts`):** `focusOrder: string[]` on `WorkspaceState`, seeded in
  `materializeEngineSnapshot` via `engine.focusRefIds()` (also covers coarse/fullResync).
  `drainEvents` re-derives it once per non-empty drain with
  `setState("focusOrder", reconcile(engine.focusRefIds()))` — one call covers both
  `focusChanged` and any item lifecycle/add/remove that changes visibility. `focusChanged`
  needs no `dispatch` case (unknown kinds fall through; `search.apply` ignores it). New
  `DocApp` methods `addToFocus`/`removeFromFocus`/`moveInFocus` (default add index =
  `state.focusOrder.length`).
- **Focus lens render:** reuses the **list-view `Dnd` path**, not a new component —
  `items()` gets a `kind==="focus"` branch, `onReorder` a focus branch → `moveInFocus`,
  `reorder` enabled for focus. So selection/expansion/open-dialog/global shortcuts (x, ⌫,
  Enter, ⌘C) all work in Focus for free. Draft/paste/Space-add stay list-only (already
  gated). Focus added to the `[`/`]` bracket-nav sequence.
- **Affordances (`Row.tsx`):** Row self-serves Focus membership from
  `props.app.state.focusOrder` (no threading). Open list/board rows show a hover/selected
  **target-icon pin toggle** (lit when focused) + a context-menu "Add/Remove from Focus".
  Focus-lens rows instead show an **× remove** button + context items "Remove from Focus"
  and a **toggle-Live** ("Mark as live"/"Mark as backlog", since the lens is flat/no-lanes).
  `TaskDialog` header carries the same pin toggle (Open items only). New icon
  `icons/target.svg`; new i18n `focus.*` group; CSS incl. `--focus-warn` soft-cap tint.
- **Nav:** static **Focus** entry (target icon) between Inbox and Done, count badge =
  `focusCount` (new prop = `state.focusOrder.length`), soft-cap colour shift at
  `FOCUS_SOFT_CAP = 9` via `data-soft-cap`. **Decided the Phase-3 UI open question:**
  soft threshold = **9** visible refs, treatment = amber count + bold (gentle, no hard cap).
- *Watch-outs for Phase 4/5:* focus toggles act on a **single item** (not the multi-select),
  by design. `moveInFocus` is driven through `planReorderMoves` exactly like `moveItem`
  (assumes the same visible-index semantics). The two-tab convergence + fingerprint-parity
  verification is deferred to Phase 5 (needs CLI focus from Phase 4).

**Phase 4 — CLI/FFI (done).**
- **CLI (`cli/src/commands/focus.rs`, new):** `monoplan focus` (list, default action),
  `focus add <item> [pos]`, `focus rm <item>`, `focus mv <item> <pos>`. Bare `focus`
  lists the view via `Option<FocusCmd>` (None ⇒ list) — same optional-subcommand shape
  clap gives `lists`. **Positions on the CLI are 1-based** (matches the numbered listing
  the user sees); converted to the doc layer's 0-based visible index via
  `one_based_to_index` (`add` with no pos ⇒ `usize::MAX` append). List output shows
  `<pos>  <id>  [mark] text` where `*` = Live (Focus is Open-only, so Live is the only
  lifecycle distinction). `--json` global emits `{pos,id,text,list_id,live}`. Wired into
  `commands/mod.rs` (`Cmd::Focus` between `Edit` and `Lists`).
- **FFI (`core/ffi/src/lib.rs`):** added `add_to_focus(item, index: Option<u32>)`
  (None ⇒ append), `remove_from_focus(item)`, `move_in_focus(item, index: u32)`,
  `focus_view() -> Vec<ItemView>` to `MonoplanStore`, mirroring the existing
  add_item/set_item_done pattern (lock → mutate → `persist`). Index is 0-based across the
  boundary (the CLI's 1-based sugar is CLI-only).
- **Integration test (`cli/tests/sync_smoke.rs`):**
  `focus_curation_converges_across_devices` — device A curates (add ×3, reorder), B pulls
  and observes the identical focus order + matching `fingerprint()`; then A marks a
  focused item Done (auto-removes the ref) and un-dones it (does **not** reappear), and B
  reconverges with matching fingerprint. This is the CLI↔web parity check the plan called
  for (the CLI uses the same `monoplan-core` that compiles to the web wasm).
- **Pre-existing bug fixed in passing:** the Phase 1 `main`→`inbox` rename left
  `core/ffi/src/lib.rs`'s `persists_across_reopen` test seeding items into a now-unreserved
  `"main"` list (`add_item` → `list not found: main`); it was never run in Phases 1–3
  (`cargo test -p monoplan-ffi` wasn't in the loop). Flipped the test's `"main"` → `"inbox"`
  and corrected the stale `all_lists` doc comment. **Watch-out:** `cargo test --workspace`
  still fails to *compile* `monoplan-core-web` off-target (E0308, pre-existing wasm-only
  crate, Phase 2 note) — run `cargo test --workspace --exclude monoplan-core-web` (21 test
  binaries green) or build web via `bun run build:wasm`.

**Phase 5 — Verify (done); cutover pending Daniel.**
- Automated: the Phase 4 integration test covers convergence + fingerprint parity +
  self-clean-on-Done + un-done-does-not-reappear.
- Manual server-backed drive (throwaway server on `:8899` + temp sqlite, to avoid the
  running `:8000` dev server): signed up device A, drove every command
  (`add`/`focus add`/`focus`/`focus mv`/`focus rm`/`focus --json` + `done`/`backlog`
  lifecycle interplay), then logged in device B and confirmed **bidirectional** pull/push
  convergence over real WS. All behaviours correct (Done auto-removes, un-done stays out,
  re-add is a no-op, 1-based positions, `*` Live marker, `list_id:"inbox"`).
- ⏳ **Not done (by design — Daniel triggers):** the live-doc export → wipe → import
  cutover (`main`→`inbox`; handoff note 5). Dry-run on a scratch account first; the agent
  must not run the production wipe.

**Decisions locked (see "Open decisions" above):** auto-remove-on-Done · no-op re-add ·
additive-v2 · bare-uuid FocusRef · **soft-cap = 9 (Phase 3 UI call, was open #5)**.

**Handoff notes / watch-outs:**
1. `reconcile()` is exposed to WASM but **never called** in web/CLI (only unit tests).
   It's a backstop only; Focus stays bounded via auto-remove-on-Done + the folded sweep.
   Don't rely on reconcile for correctness.
2. `FocusChanged` carries no payload by design — the web store must re-derive
   `focus_view()` on item events too (a Done/Bin elsewhere changes focus visibility).
3. WASM has **two** handle structs; every prior method appears twice. Add focus methods
   to both, or CLI-vs-web parity silently breaks.
4. Fingerprint now includes focus order → any web/CLI convergence check will compare it;
   this is intended.
5. **Cutover sequencing (important):** the plan said "land the cutover at the Phase 1
   checkpoint," but that would break the live web app, which still has `id:"main"`
   literals until Phase 3. The cutover is therefore deferred to **Phase 5**, after web +
   CLI speak `inbox`. Dry-run on a scratch account; **Daniel runs the wipe**, not the
   agent.

---

## Open decisions — RESOLVED (2026-07-12, confirmed with Daniel)

1. **Lifecycle interplay — RESOLVED: auto-remove focus ref on Done** (reverses the
   B.5 keep-and-filter recommendation). Investigation found: (a) a Done focus ref
   renders *nothing* (Focus view is Open-only), unlike a Done order entry that still
   renders in the board's Done lane — so the "mirror order-container discipline"
   analogy doesn't transfer; (b) `reconcile()` is **not wired to run** in web or CLI
   (only unit tests call it), so keep-and-filter would accumulate dead refs without
   bound. So: the **Done** transition removes the item's focus ref in the same commit
   (the one lifecycle→second-container write); **Bin** stays filter-only and is swept
   on the next focus interaction; **un-done does NOT reappear** in Focus (re-add
   deliberately). Focus mutations (`add`/`remove`/`move`) fold a dead-ref sweep into
   their commit. See `spec/focus.md` "Lifecycle interplay". *Phase 1 tests change
   accordingly: replace "un-done-reappears" with "done-auto-removes-ref" and
   "un-done-does-not-reappear".*
2. **Adding an already-focused item — RESOLVED: no-op** (don't move-to-top).
3. **Schema — RESOLVED: additive within v2** (no version bump) for `focus`.
4. ~~i18n key `home` → `inbox`~~ **Resolved: full flip including the stored id** —
   `main` → `inbox` everywhere, with a one-time export/wipe/import cutover on the live
   doc (Part A).
5. **Soft-cap threshold** (~7–10) and its visual treatment (B.11) — **RESOLVED in
   Phase 3: threshold = 9 visible refs; treatment = amber + bold count badge** (gentle,
   no hard cap). `FOCUS_SOFT_CAP` in `nav.tsx`, `--focus-warn` in `styles.css`.
6. **FocusRef — RESOLVED: bare uuid now** (parser accepts colon-less as local doc;
   `<doc_id>:<item_id>` reserved for cross-doc).
</content>
</invoke>
