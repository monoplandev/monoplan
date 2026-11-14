//! `find <query>`: the search index (`spec/search.md`) from the command
//! line. The search, the merge across workspaces and the ranking are
//! `api::find`, which the native clients' search field reads too; this
//! is the arguments and the rendering.

use std::io::Write;

use clap::Parser;
use serde::{Deserialize, Serialize};

use super::items::write_json;
use crate::api::{self, Ctx, find::FindKind};
use crate::workspace::HOME_LABEL;

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

/// `find` over every open doc (`api::find::find`), Home first; rows
/// carry their workspace where the caller knows its workspaces.
pub fn run(cx: &Ctx, args: FindArgs, out: &mut dyn Write) -> anyhow::Result<()> {
    let hits = api::find::find(cx, &args.query, args.limit)?;
    if args.json {
        return write_json(out, &hits);
    }
    for h in &hits {
        let kind = match h.kind {
            FindKind::List => "list",
            FindKind::Item => "item",
        };
        write!(out, "{kind}  {}  {}", h.id, h.title)?;
        if let Some(list) = &h.list_name {
            write!(out, "  ({list})")?;
        }
        if h.archived {
            write!(out, "  (archived)")?;
        }
        if h.doc_id.is_some() {
            write!(out, "  [{}]", h.doc_name.as_deref().unwrap_or(HOME_LABEL))?;
        }
        writeln!(out)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::Candidate;
    use monoplan_core::{Doc, DocId, LIST_INBOX};
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
        let lens = crate::commands::focus::Lens::home(&home);
        let cx = Ctx {
            doc: &home,
            open_docs: &[&home, &work],
            lens: &lens,
            workspaces: &docs,
            home: Some(home_id),
        };
        let run = |query: &str| {
            let mut out = Vec::new();
            run(
                &cx,
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
