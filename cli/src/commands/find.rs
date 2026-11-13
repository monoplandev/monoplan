//! `find <query>`: the search index (`spec/search.md`) from the command
//! line, and what the native clients' search field reads.
//!
//! The engine is the core's (`Doc::search`: AND across tokens, last
//! token a prefix, its ranking). This is the surface's half, the same
//! policy as the web's Find (`js/web/src/findResults.tsx`): fan out over
//! every open doc, merge by score (stable, so ties keep doc order, Home
//! first), float lists above items, and tag each row with its workspace,
//! its list's name and whether that list is archived. An empty query
//! finds nothing. The built-in views (Starred, Agenda, Done, Home's
//! Inbox) are not doc entities; a client matches their labels itself
//! with the core's `matches_name`.

use std::collections::HashMap;
use std::io::Write;

use clap::Parser;
use monoplan_core::{Doc, LIST_EVENTS, LIST_INBOX, ListView, SearchKind};
use serde::{Deserialize, Serialize};

use super::items::write_json;
use crate::workspace::{Candidate, HOME_LABEL};

#[derive(Parser, Debug, Clone, Serialize, Deserialize)]
pub struct FindArgs {
    /// What to find: every word must match, the last one as a prefix.
    pub query: String,
    /// At most this many results.
    #[arg(short = 'n', long = "limit", default_value_t = 50)]
    pub limit: usize,
    /// Machine-parseable output.
    #[arg(long)]
    pub json: bool,
}

#[derive(Serialize)]
struct HitJson<'a> {
    /// `item` or `list`.
    kind: SearchKind,
    id: String,
    title: String,
    /// The row's workspace (`doc_name` absent for Home and an unnamed
    /// doc). Absent only where the command ran on a bare doc.
    #[serde(skip_serializing_if = "Option::is_none")]
    doc_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    doc_name: Option<&'a str>,
    /// Items: the list the item is in, and its name there (absent when
    /// the list is gone).
    #[serde(skip_serializing_if = "Option::is_none")]
    list_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    list_name: Option<String>,
    /// Items: the lifecycle's name, `binned` included.
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<&'static str>,
    /// Lists: the list's emoji, when it has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    icon: Option<String>,
    /// The list (or the item's list) is archived.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    archived: bool,
    score: u64,
}

/// The merged, ranked hits over `docs`, at most `limit`.
fn hits<'a>(docs: &[(Option<&'a Candidate>, &Doc)], query: &str, limit: usize) -> Vec<HitJson<'a>> {
    let mut out: Vec<HitJson<'a>> = Vec::new();
    for (tag, doc) in docs {
        let found = doc.search(query, limit);
        if found.is_empty() {
            continue;
        }
        // Archived lists included: they stay indexed and keep naming
        // their items. The reserved lists have no `ListMeta` row, so
        // they are named here as the clients label them.
        let lists: HashMap<String, ListView> = doc
            .all_lists()
            .into_iter()
            .map(|l| (l.id.clone(), l))
            .collect();
        for r in found {
            let list = match r.kind {
                SearchKind::List => lists.get(&r.id),
                SearchKind::Item => r.list_id.as_ref().and_then(|id| lists.get(id)),
            };
            let list_name = match r.list_id.as_deref() {
                None => None,
                Some(LIST_INBOX) => Some("Inbox".to_string()),
                Some(LIST_EVENTS) => Some("Events".to_string()),
                Some(_) => list.map(|l| l.name.clone()),
            };
            out.push(HitJson {
                kind: r.kind,
                id: r.id,
                title: r.title,
                doc_id: tag.map(|c| c.doc_id.0.to_string()),
                doc_name: tag.and_then(|c| c.name.as_deref()),
                list_id: r.list_id,
                list_name,
                state: r.lifecycle.map(|l| l.name()),
                icon: match r.kind {
                    SearchKind::List => list.and_then(|l| l.icon.clone()),
                    SearchKind::Item => None,
                },
                archived: list.is_some_and(|l| l.archived_at.is_some()),
                score: r.score,
            });
        }
    }
    // Lists above items, each group by relevance. Stable, so ties
    // keep doc order.
    out.sort_by_key(|h| {
        let group = match h.kind {
            SearchKind::List => 0,
            SearchKind::Item => 1,
        };
        (group, std::cmp::Reverse(h.score))
    });
    out.truncate(limit);
    out
}

fn print(hits: &[HitJson], json: bool, out: &mut dyn Write) -> anyhow::Result<()> {
    if json {
        return write_json(out, &hits);
    }
    for h in hits {
        let kind = match h.kind {
            SearchKind::List => "list",
            SearchKind::Item => "item",
        };
        write!(out, "{kind}  {}  {}", h.id, h.title)?;
        if let Some(list) = &h.list_name {
            write!(out, "  ({list})")?;
        }
        if h.archived {
            write!(out, "  (archived)")?;
        }
        if h.doc_id.is_some() {
            write!(out, "  [{}]", h.doc_name.unwrap_or(HOME_LABEL))?;
        }
        writeln!(out)?;
    }
    Ok(())
}

/// `find` over one bare doc: rows carry no workspace.
pub fn run(doc: &Doc, args: FindArgs, out: &mut dyn Write) -> anyhow::Result<()> {
    print(
        &hits(&[(None, doc)], &args.query, args.limit),
        args.json,
        out,
    )
}

/// `find` as the daemon serves it: over `docs`, every workspace the
/// install has open, Home first.
pub fn run_all(
    docs: &[(Candidate, &Doc)],
    args: FindArgs,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    let docs: Vec<(Option<&Candidate>, &Doc)> = docs.iter().map(|(c, d)| (Some(c), *d)).collect();
    print(&hits(&docs, &args.query, args.limit), args.json, out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use monoplan_core::DocId;
    use uuid::Uuid;

    #[test]
    fn find_merges_open_docs_lists_first_and_tags_rows() {
        let home = Doc::new().unwrap();
        let work = Doc::new().unwrap();
        let groceries = home.add_list("Groceries").unwrap();
        home.add_item(&groceries, "Buy milk").unwrap();
        home.add_item(LIST_INBOX, "Groceries run").unwrap();
        let old = work.add_list("Old").unwrap();
        work.add_item(&old, "Groceries budget").unwrap();
        work.set_list_archived(&old, true).unwrap();
        let home_id = DocId(Uuid::from_u128(1));
        let work_id = DocId(Uuid::from_u128(2));
        let docs = [
            (
                Candidate {
                    doc_id: home_id,
                    name: None,
                },
                &home,
            ),
            (
                Candidate {
                    doc_id: work_id,
                    name: Some("Work".into()),
                },
                &work,
            ),
        ];
        let run = |query: &str| {
            let mut out = Vec::new();
            run_all(
                &docs,
                FindArgs {
                    query: query.into(),
                    limit: 50,
                    json: true,
                },
                &mut out,
            )
            .unwrap();
            serde_json::from_slice::<serde_json::Value>(&out).unwrap()
        };

        let rows = run("groc");
        let rows = rows.as_array().unwrap();
        // The list leads, then the title hits of both docs, then the
        // item that matches only through its list's name.
        assert_eq!(rows[0]["kind"], "list");
        assert_eq!(rows[0]["title"], "Groceries");
        assert_eq!(rows[0]["doc_id"], home_id.0.to_string());
        assert!(rows[0].get("doc_name").is_none());
        let titles: Vec<&str> = rows[1..]
            .iter()
            .map(|r| r["title"].as_str().unwrap())
            .collect();
        assert_eq!(titles, ["Groceries run", "Groceries budget", "Buy milk"]);
        let run_row = &rows[1];
        assert_eq!(run_row["kind"], "item");
        assert_eq!(run_row["list_name"], "Inbox");
        assert_eq!(run_row["state"], "backlog");
        assert!(run_row.get("archived").is_none());
        let budget = &rows[2];
        assert_eq!(budget["doc_name"], "Work");
        assert_eq!(budget["list_name"], "Old");
        assert_eq!(budget["archived"], true);

        assert!(run("").as_array().unwrap().is_empty());
        assert!(run("nothing").as_array().unwrap().is_empty());
    }
}
