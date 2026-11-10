# Known bugs

Reproduced defects left unfixed for now. Remove an entry when it is fixed.

## A no-op call inside an action batch over-counts undo steps (web)

Found 2026-11-11. Not fixed.

**Symptom.** One Cmd+Z undoes a batched action *and* the action before it.

**Cause** (`js/web/src/sync/store.ts`). Outside a batch, `mutate` records an
undo entry only if the doc changed. Inside `withActionBatch`, every `mutate`
call adds one step to the batch's entry whether or not the core committed
anything. Undo then calls `replica.undo()` once per counted step, and each
call pops one real core commit, so every no-op call in the batch makes undo
reach one commit too far.

**Reproduction.** List `a, b, c`; rename `a`; then in one batch
`moveItem(a, "inbox", 0)` (already first: the core returns without
committing) and `moveItem(c, "inbox", 1)` (commits). The entry says 2 steps
and holds 1 commit. One undo puts `c` back and also reverts the rename.

**Reachable from.** The nav drop and the move palette, which loop
`moveItem(id, list, i)` in a batch: a multi-selection where one row is
already in its target slot. Other batches (`undoneIntoFocus`, …) are not
audited.

**Worked around in.** `DocsLayer.moveToDoc` calls `retargetFocus` only when a
moved item is starred, since it commits nothing otherwise.

**Likely fix.** In a batch, count a step only when the call changed the doc
(as the non-batch path already checks). Not tried.
