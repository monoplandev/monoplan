//! Attachments on the engine loop (`spec/attachments.md` phase 4): the
//! host half in `monoplan_cli::files` against a real server. Attach,
//! open, remove (the loop's housekeeping destroys the blob once the
//! removal has synced), and the blob copy around a cross-doc move.

mod support;

use std::time::Duration;

use monoplan_cli::api;
use monoplan_cli::api::moves::ItemRef;
use monoplan_cli::daemon::{self, LocalClient};
use monoplan_cli::files;
use monoplan_cli::ipc::{Reply, Request};
use monoplan_core::LIST_INBOX;
use monoplan_protocol::FilesListResponse;
use support::{TestServer, http, materialize_profile, reopen_profile, signup_account};

async fn listed(base: &str, token: &str, doc: &str) -> FilesListResponse {
    http()
        .get(format!("{base}/api/docs/{doc}/files"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .bytes()
        .await
        .map(|bytes| serde_json::from_slice(&bytes).unwrap())
        .unwrap()
}

async fn add_item(local: &LocalClient, workspace: Option<String>, text: &str) -> String {
    let text = text.to_string();
    local
        .call(workspace, move |cx| {
            Ok(api::items::add_items(cx, LIST_INBOX, &[text], false)?.remove(0))
        })
        .await
        .unwrap()
}

async fn files_of(local: &LocalClient, workspace: Option<String>, id: &str) -> Vec<String> {
    let id = id.to_string();
    local
        .call(workspace, move |cx| {
            Ok(api::items::item(cx, &id)?
                .files
                .into_iter()
                .map(|f| f.id)
                .collect())
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn attach_open_remove_and_move_across_workspaces() {
    let server = TestServer::start().await;
    let signup = signup_account(&server, "files-a").await;
    let tmp = tempfile::tempdir().unwrap();
    let profile = materialize_profile(
        tmp.path(),
        &server.base,
        &signup.account_id,
        &signup.primary_doc_id,
        &signup.device_id,
        &signup.device_token,
        &signup.dek,
        &signup.email,
        true,
    )
    .await;
    let mut handle = daemon::spawn(profile).await.unwrap();
    let local = handle.client.clone();
    let profile = reopen_profile(tmp.path());
    let home = signup.primary_doc_id.clone();
    let token = signup.device_token.clone();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !handle.status.borrow().online {
            handle.status.changed().await.unwrap();
        }
    })
    .await
    .expect("daemon online");

    // Attach: the blob is on the server, the entry on the item.
    let scratch = tempfile::tempdir().unwrap();
    let source = scratch.path().join("note.txt");
    let body = b"the plaintext of an attachment".to_vec();
    std::fs::write(&source, &body).unwrap();
    let item = add_item(&local, None, "with a file").await;
    let file_id = files::attach(
        &local,
        &profile,
        None,
        item.clone(),
        &source,
        " note\u{0}.txt ",
        "text/plain",
    )
    .await
    .unwrap();
    assert_eq!(files_of(&local, None, &item).await, vec![file_id.clone()]);
    let info = {
        let item = item.clone();
        local
            .call(None, move |cx| api::items::item(cx, &item))
            .await
            .unwrap()
    };
    assert_eq!(info.files[0].name, "note.txt");
    assert_eq!(info.files[0].mime, "text/plain");
    assert_eq!(info.files[0].size, body.len() as u64);
    let rows = listed(&server.base, &token, &home).await.files;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, file_id);
    // Sealed: nonce and tag on top of the plaintext.
    assert_eq!(rows[0].bytes, body.len() as u64 + 40);

    // Open: the plaintext comes back.
    let dest = scratch.path().join("opened");
    files::fetch(&local, &profile, None, file_id.clone(), &dest)
        .await
        .unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), body);

    // An item that is not there is refused before anything is sent.
    let missing = files::attach(
        &local,
        &profile,
        None,
        "0".repeat(32),
        &source,
        "note.txt",
        "",
    )
    .await;
    assert!(missing.is_err());
    assert_eq!(listed(&server.base, &token, &home).await.files.len(), 1);

    // Cross-doc move: the blob is copied first, the entry travels with
    // the item, and the source's blob is tombstoned by the purge.
    let work = match local
        .request(Request::CreateWorkspace {
            name: "Work".into(),
        })
        .await
    {
        Reply::WorkspaceCreated { doc_id, .. } => doc_id,
        other => panic!("unexpected: {other:?}"),
    };
    let moving = ItemRef {
        workspace: None,
        id: item.clone(),
    };
    files::copy_for_move(&local, &profile, vec![moving.clone()], Some(work.clone()))
        .await
        .unwrap();
    {
        let work = work.clone();
        local
            .call(None, move |cx| {
                api::moves::move_item_across(cx, &moving, Some(&work), LIST_INBOX)
            })
            .await
            .unwrap();
    }
    assert_eq!(
        files_of(&local, Some(work.clone()), &item).await,
        vec![file_id.clone()]
    );
    let dest = scratch.path().join("opened-after-move");
    files::fetch(&local, &profile, Some(work.clone()), file_id.clone(), &dest)
        .await
        .unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), body);
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let rows = listed(&server.base, &token, &home).await.files;
            if rows.len() == 1 && rows[0].deleted_at.is_some() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the released blob is tombstoned in the source doc");

    // Remove: permanent, and the server's row and bytes go once the
    // removal has synced, with the purge mark forgotten after.
    {
        let file_id = file_id.clone();
        local
            .call(Some(work.clone()), move |cx| {
                api::items::remove_file(cx, &file_id)
            })
            .await
            .unwrap();
    }
    assert!(files_of(&local, Some(work.clone()), &item).await.is_empty());
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if listed(&server.base, &token, &work).await.files.is_empty() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the removed blob is destroyed");
    assert!(
        files::fetch(&local, &profile, Some(work.clone()), file_id, &dest)
            .await
            .is_err()
    );

    handle.shutdown().await.unwrap();
}
