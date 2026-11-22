# Subtasks

Status: **first pass built 2026-11-18: core, wasm and web; the macOS item panel and the iOS item page 2026-11-19. Not on the CLI.**

**Vetoed (2026-08-27): subtasks as full items** (an item located under a
parent item, with its own lifecycle, location and lens membership). It hides
things too well. The explored design is in git history
(`df2e4fbc:spec/subtasks-plan.md`).

**Direction (2026-11-18): a subtask is a text node with done / not done**,
owned by its parent item. Shopping lists, travel checklists. First class (its
own structure on the item and its own rows in the item surface, with a count
on the parent's row), not checkboxes inside notes: a text editor is a poor
place to tick things on a phone. Not an item: no location, no dates, no star,
no assignee, no row in any lens.

**Vetoed (2026-11-18): subtasks shown in lists.** Subtasks sit only in the
item details page. A list, board or lens never renders them, expanded under
the parent or otherwise; the most a parent's row carries is a count.

Open: whether a subtask records a done timestamp (leaning no; not stored).

## As built (2026-11-18, first pass)

- **Core.** `items/<id>/subtasks`, a mergeable `LoroMovableList` of atomic
  `{id, text, done?}` maps; the register row in `data-model.md` is the
  contract. `Doc::add_subtask` / `set_subtask_text` / `set_subtask_done` /
  `move_subtask` / `remove_subtask`, `ItemView.subtasks`,
  `AppEvent::ItemSubtasksChanged` (the whole list, on local writes, remote
  imports and undo alike; `ItemAdded` carries the list too). The diff
  classifier routes the list's diffs by event path, as it does `notes`.
- **Wasm.** `addSubtask` / `setSubtaskText` / `setSubtaskDone` /
  `moveSubtask` / `removeSubtask`; `subtasks` crosses as JSON text on
  `itemAdded` / `itemSubtasksChanged` and as an array in the snapshot.
- **Web.** `SubtasksField` in the task surface between the place and the
  notes, existing items only (a capture gains it once its title blur
  promotes it): a checkbox and a text field per subtask, a
  hover grip at the row's end, and a trailing "Add subtask" field that
  keeps the caret after Enter. Enter commits and moves down, Backspace in
  an emptied row removes it (the only way to remove one: the hover ✕ was
  dropped 2026-11-19), Up / Down walk the rows, Alt+Up / Alt+Down
  reorder, Escape puts the text back. Dragging the grip reorders
  (2026-11-19): the row follows the pointer inside the list, the rows it
  passes step aside, the release is one `moveSubtask`, Escape cancels.
  Long text wraps (2026-11-22): fields are textareas sized to their
  text, the checkbox and grip sit on the first line, and the grip drag
  measures each row's height. The text stays one line: Enter commits,
  a pasted newline becomes a space. Up / Down walk a wrapped row's lines
  before leaving it from its start / end. List rows and board cards show a `done/total` count on open
  items, nothing else. Duplicate copies subtasks with their ticks.

- **Typed api and FFI (2026-11-19).** `api::items::Subtask`,
  `ItemInfo.subtasks` (in `item show --json` when the item has any),
  `api::items::{add_subtask, set_subtask_text, set_subtask_done,
  move_subtask, remove_subtask}` and the five `Engine` methods of the
  same names. No CLI command writes subtasks.
- **macOS (2026-11-19).** `SubtasksField.swift` in the item panel
  between the place and the notes, the web's field key for key (Option
  for Alt): checkbox, one-line field, hover grip, trailing "Add
  subtask". The grip drag is plain SwiftUI offsets over rows of one
  height, not a table or a system drag session. A reorder is shown at
  once and held until the reread brings it. Backspace removes an
  emptied row on a fresh press only: AppKit selects a field's text as
  it takes the focus, so a held key would otherwise run up the list. A
  binned item's subtasks are shown, not edited. No count on the Mac's
  rows yet. Long text wraps (2026-11-23), as on the web: vertical-axis
  fields, the checkbox and grip on the first line, the grip drag over
  each row's measured height. Return (any modifier) commits; a pasted
  newline becomes a space. Up / Down walk a wrapped row's lines, read
  off the window's field editor, before leaving from its start / end.
- **iOS (2026-11-19).** `ItemSubtasksField.swift` on the item page
  between the place and the notes, edited in place (no editor takes the
  sheet): the Mac's field for touch, rows a full 44 pt, the grip always
  shown. A row emptied and then left (Return, or the focus going) is
  removed: the software keyboard reports no Backspace in an empty
  field. A hardware keyboard gets the Mac's keys. The page now puts the
  keyboard away on a scroll. No count on iOS rows yet. Long text wraps
  (2026-11-23), the Mac's way; a software keyboard's Return arrives as
  a newline in the field and is taken as the commit, and a hardware
  keyboard's Up / Down read the caret from the field's `TextSelection`.

Not built: "untick all" for a re-used checklist, subtask
text in search, a count on the calendar's rows or any Mac or iOS row, CLI.
