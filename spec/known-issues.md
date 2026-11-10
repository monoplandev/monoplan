# Known Issues

## Reorder undo/redo

Grouped movable-list reorder undo is unsafe in the current core/Loro path.

- Repro: perform a larger reorder as many `move_item(...)` calls inside one undo group, then `undo()`, then `redo()`.
- Observed behavior: redo can corrupt list order and effectively drop/eat trailing items.
- This reproduces in `monoplan-core` without the web UI, so it is not a projection/render bug.
- The failed `moveItems` experiment was removed. It did not solve the problem and had the same underlying undo-safety issue.

### Current workaround

For launch, reorder does **not** use core undo grouping. The grouped-undo API was removed from `monoplan-core` rather than left around as an attractive footgun.

- Reorder still executes as plain per-item `move_item(...)` mutations, but
  only for the selected rows. A drag's commit count is bounded by selection
  size and does not grow with distance travelled.
- The web app keeps a thin app-level action-batch stack that records how many plain core undo steps belong to one visible reorder action.
- One user undo/redo for reorder replays `replica.undo()` / `replica.redo()` that many times, then renders once at the end.
- Undo/redo captures Loro's container diffs and translates them into surgical
  app events. Unsupported or bulk diff shapes retain a whole-doc fallback.

### Scope note

This workaround appears acceptable for Monoplan's task/list UX, but it should not be treated as a durable pattern for higher-stakes domains. Generic grouped undo for important state transitions remains conceptually and technically suspect.
