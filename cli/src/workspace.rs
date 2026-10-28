//! Workspace selection: the global `-w <workspace>` (`spec/cli.md`
//! §Workspaces). A selector names a doc by its `doc_name` or its full
//! doc id; absent, `personal` and `home` are the Home doc, which is
//! never named inside the doc. Resolution is a pure function over the
//! `(doc id, name)` pairs a caller has: the daemon's open engines, or
//! the docs a client loads from storage when it runs without one.

use monoplan_core::DocId;

/// Spellings of the Home doc, matched case-insensitively.
const HOME_ALIASES: [&str; 2] = ["personal", "home"];
/// What Home is called in listings and errors (it has no `doc_name`).
pub const HOME_LABEL: &str = "Personal";

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error("no workspace named {wanted:?}; this install has: {have}")]
    NotFound { wanted: String, have: String },
    #[error("{wanted:?} names more than one workspace ({ids}); use the doc id")]
    Ambiguous { wanted: String, ids: String },
}

/// One doc this install can open, as the resolver sees it.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub doc_id: DocId,
    /// `doc_name`, `None` for Home and for an unnamed doc.
    pub name: Option<String>,
}

/// The doc `selector` names among `candidates`, which must include
/// `primary`. `None` is Home.
pub fn resolve(
    primary: DocId,
    candidates: &[Candidate],
    selector: Option<&str>,
) -> Result<DocId, WorkspaceError> {
    let Some(raw) = selector else {
        return Ok(primary);
    };
    let wanted = raw.trim();
    if wanted.is_empty() || HOME_ALIASES.iter().any(|a| a.eq_ignore_ascii_case(wanted)) {
        return Ok(primary);
    }
    if let Ok(id) = uuid::Uuid::parse_str(wanted) {
        let id = DocId(id);
        if candidates.iter().any(|c| c.doc_id == id) {
            return Ok(id);
        }
    }
    let matches: Vec<&Candidate> = candidates
        .iter()
        .filter(|c| {
            c.name
                .as_deref()
                .is_some_and(|n| n.eq_ignore_ascii_case(wanted))
        })
        .collect();
    match matches.as_slice() {
        [one] => Ok(one.doc_id),
        [] => Err(WorkspaceError::NotFound {
            wanted: wanted.to_string(),
            have: describe(primary, candidates),
        }),
        many => Err(WorkspaceError::Ambiguous {
            wanted: wanted.to_string(),
            ids: many
                .iter()
                .map(|c| c.doc_id.0.to_string())
                .collect::<Vec<_>>()
                .join(", "),
        }),
    }
}

/// `Personal, Work, <unnamed id>` — what an error lists.
fn describe(primary: DocId, candidates: &[Candidate]) -> String {
    let mut names = vec![HOME_LABEL.to_string()];
    for c in candidates {
        if c.doc_id == primary {
            continue;
        }
        names.push(match &c.name {
            Some(n) => n.clone(),
            None => format!("(unnamed) {}", c.doc_id.0),
        });
    }
    names.join(", ")
}

/// Resolve a selector without a daemon: load each doc this install
/// holds a key for just far enough to read its name. Read-only, so it
/// is safe beside a running daemon; used by the verbs that never start
/// one (`workspace status`, `workspace share …`).
pub fn resolve_offline(
    storage: &crate::storage::SqliteStorage,
    secrets: &crate::config::Secrets,
    account: &crate::storage::Account,
    selector: Option<&str>,
) -> anyhow::Result<DocId> {
    let primary = account.primary_doc_id;
    let home_only = [Candidate {
        doc_id: primary,
        name: None,
    }];
    // Cheap paths first: no selector or a Home alias.
    if let Ok(id) = resolve(primary, &home_only, selector) {
        return Ok(id);
    }
    let mut candidates = home_only.to_vec();
    for key in secrets.docs.keys() {
        let Ok(doc_id) = crate::keystore::parse_doc_id(key) else {
            continue;
        };
        if doc_id == primary {
            continue;
        }
        // An id we hold: no need to load anything.
        if let Some(sel) = selector
            && uuid::Uuid::parse_str(sel.trim()).is_ok_and(|u| u == doc_id.0)
        {
            return Ok(doc_id);
        }
        let dek = crate::keystore::dek_for_doc(secrets, doc_id)?;
        let doc = crate::storage::load_doc(storage, &dek, doc_id)?;
        candidates.push(Candidate {
            doc_id,
            name: doc.get_settings().doc_name,
        });
    }
    Ok(resolve(primary, &candidates, selector)?)
}

/// The display name for a doc: its `doc_name`, Home's label, or the id.
pub fn label(primary: DocId, doc_id: DocId, name: Option<&str>) -> String {
    if doc_id == primary {
        return HOME_LABEL.to_string();
    }
    match name {
        Some(n) => n.to_string(),
        None => doc_id.0.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u8) -> DocId {
        DocId(uuid::Uuid::from_bytes([n; 16]))
    }

    fn candidates() -> Vec<Candidate> {
        vec![
            Candidate {
                doc_id: id(1),
                name: None,
            },
            Candidate {
                doc_id: id(2),
                name: Some("Work".into()),
            },
            Candidate {
                doc_id: id(3),
                name: None,
            },
        ]
    }

    #[test]
    fn absent_and_aliases_are_home() {
        let c = candidates();
        assert_eq!(resolve(id(1), &c, None).unwrap(), id(1));
        assert_eq!(resolve(id(1), &c, Some("Personal")).unwrap(), id(1));
        assert_eq!(resolve(id(1), &c, Some(" home ")).unwrap(), id(1));
    }

    #[test]
    fn names_match_case_insensitively_and_ids_exactly() {
        let c = candidates();
        assert_eq!(resolve(id(1), &c, Some("work")).unwrap(), id(2));
        assert_eq!(
            resolve(id(1), &c, Some(&id(3).0.to_string())).unwrap(),
            id(3)
        );
        let e = resolve(id(1), &c, Some("Play")).unwrap_err();
        assert!(matches!(e, WorkspaceError::NotFound { .. }));
        let msg = e.to_string();
        assert!(msg.contains("Personal, Work, (unnamed)"), "{msg}");
    }

    #[test]
    fn duplicate_names_are_ambiguous() {
        let mut c = candidates();
        c[2].name = Some("WORK".into());
        let e = resolve(id(1), &c, Some("work")).unwrap_err();
        assert!(matches!(e, WorkspaceError::Ambiguous { .. }));
    }

    #[test]
    fn an_unknown_id_is_not_found() {
        let c = candidates();
        let e = resolve(id(1), &c, Some(&id(9).0.to_string())).unwrap_err();
        assert!(matches!(e, WorkspaceError::NotFound { .. }));
    }
}
