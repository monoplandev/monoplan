# Subtasks

Status: **direction only, not built.**

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

Open: whether a subtask records a done timestamp (leaning no).
