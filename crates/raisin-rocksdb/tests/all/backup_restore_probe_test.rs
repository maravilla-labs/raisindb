//! Restore a backup written by a running server and report what came back.
//!
//! Ignored: it needs a backup on disk. Used for the round trip of a real
//! server's data (users with keyed homes, FRIENDS_WITH relations, a chat):
//!
//! ```text
//! RAISIN_RESTORE_SRC=/path/to/backup RAISIN_RESTORE_TENANT=default \
//! RAISIN_RESTORE_REPO=src cargo test -p raisin-rocksdb --test all \
//!   backup_restore_probe -- --ignored --nocapture
//! ```
//!
//! It restores into a fresh store and asserts that every relation's two
//! endpoints exist there, then prints users (path, key), relations by path and
//! the conversations under user homes.

use raisin_error::Result;
use raisin_models::auth::user_key::user_key_from_home;
use raisin_rocksdb::management::backup::restore_repository;
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::{BranchScope, NodeRepository, RelationRepository, Storage, StorageScope};

#[tokio::test]
#[ignore]
async fn backup_restore_probe() -> Result<()> {
    let (Ok(src), Ok(tenant), Ok(repo)) = (
        std::env::var("RAISIN_RESTORE_SRC"),
        std::env::var("RAISIN_RESTORE_TENANT"),
        std::env::var("RAISIN_RESTORE_REPO"),
    ) else {
        eprintln!("set RAISIN_RESTORE_SRC / _TENANT / _REPO");
        return Ok(());
    };
    let dir = tempfile::TempDir::new().unwrap();
    let storage = RocksDBStorage::new(dir.path())?;
    restore_repository(&storage, &tenant, &repo, std::path::Path::new(&src)).await?;

    let ac = StorageScope::new(&tenant, &repo, "main", "raisin:access_control");
    let rels = storage
        .relations()
        .scan_relations_global(BranchScope::new(&tenant, &repo, "main"), None, None)
        .await?;
    println!("relations: {}", rels.len());
    for (sws, sid, tws, tid, rel) in &rels {
        let s = storage
            .nodes()
            .get(StorageScope::new(&tenant, &repo, "main", sws), sid, None)
            .await?
            .unwrap_or_else(|| panic!("source {sws}:{sid} restored"));
        let t = storage
            .nodes()
            .get(StorageScope::new(&tenant, &repo, "main", tws), tid, None)
            .await?
            .unwrap_or_else(|| panic!("target {tws}:{tid} restored"));
        println!(
            "  {} -[{} w={:?}]-> {}",
            s.path, rel.relation_type, rel.weight, t.path
        );
    }
    for path in ["/users/internal"] {
        let Some(folder) = storage.nodes().get_by_path(ac, path, None).await? else {
            continue;
        };
        for child in storage
            .nodes()
            .list_children(ac, &folder.path, raisin_storage::ListOptions::default())
            .await?
        {
            if child.node_type == "raisin:User" {
                println!(
                    "user {} key={:?} id={}",
                    child.path,
                    user_key_from_home(&child.path),
                    child.id
                );
                let chats = format!("{}/inbox/chats", child.path);
                if storage
                    .nodes()
                    .get_by_path(ac, &chats, None)
                    .await?
                    .is_some()
                {
                    for conv in storage
                        .nodes()
                        .list_children(ac, &chats, raisin_storage::ListOptions::default())
                        .await?
                    {
                        println!(
                            "  conversation {} participant_keys={:?}",
                            conv.path,
                            conv.properties.get("participant_keys")
                        );
                    }
                }
            }
        }
    }
    Ok(())
}
