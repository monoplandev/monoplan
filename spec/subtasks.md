# Subtasks

Status: **first pass built 2026-11-18: core, wasm and web. Not on iOS, macOS or the CLI.**

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
  promotes it): a checkbox and a one-line text field per subtask, a
  hover ✕, and a trailing "Add subtask" field that keeps the caret after
  Enter. Enter commits and moves down, Backspace in an emptied row removes
  it, Up / Down walk the rows, Alt+Up / Alt+Down reorder, Escape puts the
  text back. List rows and board cards show a `done/total` count on open
  items, nothing else. Duplicate copies subtasks with their ticks.

Not built: drag reorder, "untick all" for a re-used checklist, subtask
text in search, a count on the calendar's rows, native clients, CLI.
