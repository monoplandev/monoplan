//! One undo history across the open docs, kept by the engine loop.
//!
//! Undo itself stays per doc (each `Doc` has its own `UndoManager`,
//! one step per eligible local commit). What a user did is one call or
//! one doc command, which can commit several times and, rarely, in more
//! than one doc. This groups the steps each one added into one entry,
//! so Undo takes back the whole of the last thing done, in whichever
//! workspace it was done: what the web's docs layer
//! (`js/web/src/sync/docs.ts`) does with stamps, as a plain stack here
//! because every local write goes through the one loop.
//!
//! The record is checked against the docs before it is used: a doc
//! whose stack holds fewer steps than its entries account for has been
//! rearmed (a hard delete, a same-peer import) or closed, and its
//! entries go. An entry that also spanned another doc takes that doc's
//! history with it (`Doc::forget_undo`), since undoing one half of a
//! linked action alone would lose or duplicate what it moved.

use std::collections::HashMap;

use monoplan_core::{Doc, DocId};

/// Undo steps one action added, per doc.
type Entry = Vec<(DocId, usize)>;

#[derive(Default)]
pub struct History {
    undo: Vec<Entry>,
    redo: Vec<Entry>,
}

/// Each open doc's undo depth before an action, from [`History::before`].
pub struct Before(HashMap<DocId, usize>);

impl History {
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Call before an action that may write: settles the record and
    /// notes where each doc's stack stands.
    pub fn before(&mut self, docs: &[(DocId, &Doc)]) -> Before {
        self.reconcile(docs);
        Before(docs.iter().map(|(id, d)| (*id, d.undo_count())).collect())
    }

    /// Call after the action: whatever it added to any doc's stack is
    /// one entry, and a new entry ends what could be redone.
    pub fn record(&mut self, before: Before, docs: &[(DocId, &Doc)]) {
        let mut entry = Entry::new();
        let mut reset = Vec::new();
        for (id, doc) in docs {
            let was = before.0.get(id).copied().unwrap_or(0);
            let now = doc.undo_count();
            if now > was {
                entry.push((*id, now - was));
            } else if now < was {
                // Rearmed during the action (a hard delete): nothing
                // before it is undoable, and steps after it in the same
                // action cannot be told apart, so the doc starts clean.
                reset.push(*id);
            }
        }
        if !reset.is_empty() {
            entry.retain(|(id, _)| !reset.contains(id));
            self.purge(reset, docs);
        }
        if !entry.is_empty() {
            self.undo.push(entry);
            self.redo.clear();
        }
    }

    /// Take back the newest action. `false` when there was nothing to
    /// take back.
    pub fn undo(&mut self, docs: &[(DocId, &Doc)]) -> anyhow::Result<bool> {
        self.reconcile(docs);
        Self::step(&mut self.undo, &mut self.redo, docs, |d| Ok(d.undo()?))
    }

    /// Do again the newest undone action.
    pub fn redo(&mut self, docs: &[(DocId, &Doc)]) -> anyhow::Result<bool> {
        self.reconcile(docs);
        Self::step(&mut self.redo, &mut self.undo, docs, |d| Ok(d.redo()?))
    }

    fn step(
        from: &mut Vec<Entry>,
        to: &mut Vec<Entry>,
        docs: &[(DocId, &Doc)],
        apply: impl Fn(&Doc) -> anyhow::Result<bool>,
    ) -> anyhow::Result<bool> {
        while let Some(entry) = from.pop() {
            let mut done = Entry::new();
            for (id, steps) in entry {
                let Some(doc) = lookup(docs, id) else {
                    continue;
                };
                let mut applied = 0;
                while applied < steps && apply(doc)? {
                    applied += 1;
                }
                if applied > 0 {
                    done.push((id, applied));
                }
            }
            // An entry nothing could apply is stale; try the next.
            if !done.is_empty() {
                to.push(done);
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Drop entries the docs no longer back: a doc that is gone, or
    /// whose undo or redo stack is shallower than its entries need.
    fn reconcile(&mut self, docs: &[(DocId, &Doc)]) {
        let mut need: HashMap<DocId, (usize, usize)> = HashMap::new();
        for entry in &self.undo {
            for (id, n) in entry {
                need.entry(*id).or_default().0 += n;
            }
        }
        for entry in &self.redo {
            for (id, n) in entry {
                need.entry(*id).or_default().1 += n;
            }
        }
        let bad: Vec<DocId> = need
            .into_iter()
            .filter(|(id, (undo, redo))| match lookup(docs, *id) {
                Some(d) => d.undo_count() < *undo || d.redo_count() < *redo,
                None => true,
            })
            .map(|(id, _)| id)
            .collect();
        if !bad.is_empty() {
            self.purge(bad, docs);
        }
    }

    /// Drop every entry touching `ids`. A doc that shared a dropped
    /// entry loses its whole history too, in the record and in the doc.
    fn purge(&mut self, mut ids: Vec<DocId>, docs: &[(DocId, &Doc)]) {
        let mut i = 0;
        while i < ids.len() {
            let id = ids[i];
            for stack in [&mut self.undo, &mut self.redo] {
                stack.retain(|entry| {
                    if !entry.iter().any(|(d, _)| *d == id) {
                        return true;
                    }
                    for (d, _) in entry {
                        if !ids.contains(d) {
                            ids.push(*d);
                        }
                    }
                    false
                });
            }
            i += 1;
        }
        for id in ids {
            if let Some(doc) = lookup(docs, id) {
                doc.forget_undo();
            }
        }
    }
}

fn lookup<'a>(docs: &[(DocId, &'a Doc)], id: DocId) -> Option<&'a Doc> {
    docs.iter().find(|(d, _)| *d == id).map(|(_, d)| *d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use monoplan_core::LIST_INBOX as INBOX_ID;

    fn id(n: u8) -> DocId {
        DocId(uuid::Uuid::from_bytes([n; 16]))
    }

    fn items(doc: &Doc) -> usize {
        doc.items_in_list(INBOX_ID, false).len()
    }

    #[test]
    fn one_action_of_several_commits_is_one_undo() {
        let a = Doc::new().unwrap();
        let docs = [(id(1), &a)];
        let mut h = History::default();

        let before = h.before(&docs);
        a.add_item(INBOX_ID, "one").unwrap();
        a.add_item(INBOX_ID, "two").unwrap();
        h.record(before, &docs);
        assert_eq!(items(&a), 2);

        assert!(h.undo(&docs).unwrap());
        assert_eq!(items(&a), 0);
        assert!(!h.can_undo());
        assert!(h.redo(&docs).unwrap());
        assert_eq!(items(&a), 2);
    }

    #[test]
    fn undo_follows_the_order_actions_were_taken_across_docs() {
        let a = Doc::new().unwrap();
        let b = Doc::new().unwrap();
        let docs = [(id(1), &a), (id(2), &b)];
        let mut h = History::default();

        for doc in [&a, &b, &a] {
            let before = h.before(&docs);
            doc.add_item(INBOX_ID, "x").unwrap();
            h.record(before, &docs);
        }
        assert!(h.undo(&docs).unwrap());
        assert_eq!((items(&a), items(&b)), (1, 1));
        assert!(h.undo(&docs).unwrap());
        assert_eq!((items(&a), items(&b)), (1, 0));
        assert!(h.redo(&docs).unwrap());
        assert_eq!((items(&a), items(&b)), (1, 1));
    }

    #[test]
    fn a_new_action_ends_redo() {
        let a = Doc::new().unwrap();
        let docs = [(id(1), &a)];
        let mut h = History::default();
        let before = h.before(&docs);
        a.add_item(INBOX_ID, "x").unwrap();
        h.record(before, &docs);
        assert!(h.undo(&docs).unwrap());
        assert!(h.can_redo());

        let before = h.before(&docs);
        a.add_item(INBOX_ID, "y").unwrap();
        h.record(before, &docs);
        assert!(!h.can_redo());
    }

    #[test]
    fn a_read_records_nothing() {
        let a = Doc::new().unwrap();
        let docs = [(id(1), &a)];
        let mut h = History::default();
        let before = h.before(&docs);
        let _ = items(&a);
        h.record(before, &docs);
        assert!(!h.can_undo());
    }

    #[test]
    fn a_rearmed_doc_drops_its_entries() {
        let a = Doc::new().unwrap();
        let b = Doc::new().unwrap();
        let docs = [(id(1), &a), (id(2), &b)];
        let mut h = History::default();
        for doc in [&a, &b] {
            let before = h.before(&docs);
            doc.add_item(INBOX_ID, "x").unwrap();
            h.record(before, &docs);
        }
        b.forget_undo();
        // b's entry is the newest, but b can no longer undo it: Undo
        // goes to a's instead of popping nothing.
        assert!(h.undo(&docs).unwrap());
        assert_eq!((items(&a), items(&b)), (0, 1));
        assert!(!h.can_undo());
    }

    #[test]
    fn a_closed_doc_drops_its_entries() {
        let a = Doc::new().unwrap();
        let b = Doc::new().unwrap();
        let mut h = History::default();
        {
            let docs = [(id(1), &a), (id(2), &b)];
            for doc in [&a, &b] {
                let before = h.before(&docs);
                doc.add_item(INBOX_ID, "x").unwrap();
                h.record(before, &docs);
            }
        }
        let docs = [(id(1), &a)];
        assert!(h.undo(&docs).unwrap());
        assert_eq!(items(&a), 0);
        assert!(!h.can_undo());
    }
}
