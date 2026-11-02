//! Phase 10b review: a backup of one-format records keeps their paths.
//!
//! Every writer stores a path-less `StorageNode` since Phase 10b. The export
//! decoded each blob raw as a `Node` (whose `path` is `#[serde(default)]`), so
//! every node exported with `"path": ""`; the restore then wrote that empty
//! path into NODE_PATH and no PATH_INDEX entry at all — a restored repository
//! with no reachable tree, and no error.

use crate::node_path_writer_test::{
    folder, id_at_path, path_at, repo_create, setup, tx_put, REPO, TENANT,
};
use raisin_error::Result;
use raisin_rocksdb::management::backup::{backup_repository, restore_repository};
use tempfile::TempDir;

#[tokio::test]
async fn backup_round_trip_of_one_format_records_keeps_paths() -> Result<()> {
    let (source, _source_dir) = setup().await?;
    repo_create(&source, folder("site", "/site")).await?;
    repo_create(&source, folder("page", "/site/page")).await?;
    // A newer version through the transaction path (SQL / WS): the export
    // takes the version visible at HEAD.
    tx_put(&source, &folder("page", "/site/page")).await?;

    let backup_dir = TempDir::new().unwrap();
    backup_repository(&source, TENANT, REPO, backup_dir.path()).await?;
    let exported = std::fs::read_to_string(
        backup_dir
            .path()
            .join(TENANT)
            .join(REPO)
            .join("nodes.jsonl"),
    )
    .unwrap();
    let paths: Vec<String> = exported
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap()["path"].to_string())
        .collect();
    assert!(paths.contains(&"\"/site/page\"".to_string()), "{paths:?}");
    assert!(!paths.iter().any(|p| p == "\"\""), "{paths:?}");

    let (restored, _restored_dir) = setup().await?;
    restore_repository(&restored, TENANT, REPO, backup_dir.path()).await?;
    assert_eq!(
        path_at(&restored, "page", None).await.as_deref(),
        Some("/site/page")
    );
    assert_eq!(
        id_at_path(&restored, "/site/page", None).await.as_deref(),
        Some("page")
    );
    assert_eq!(
        id_at_path(&restored, "/site", None).await.as_deref(),
        Some("site")
    );
    Ok(())
}

/// A backup taken by a release that exported path-less records holds rows
/// with `"path": ""`. Restoring one used to assert `""` in NODE_PATH; now the
/// restore refuses before writing anything.
#[tokio::test]
async fn restore_refuses_nodes_without_a_path() -> Result<()> {
    let (source, _source_dir) = setup().await?;
    tx_put(&source, &folder("page", "/page")).await?;
    let backup_dir = TempDir::new().unwrap();
    backup_repository(&source, TENANT, REPO, backup_dir.path()).await?;
    let nodes_file = backup_dir
        .path()
        .join(TENANT)
        .join(REPO)
        .join("nodes.jsonl");
    let blanked: String = std::fs::read_to_string(&nodes_file)
        .unwrap()
        .lines()
        .map(|line| {
            let mut node: serde_json::Value = serde_json::from_str(line).unwrap();
            node["path"] = serde_json::json!("");
            format!("{node}\n")
        })
        .collect();
    std::fs::write(&nodes_file, blanked).unwrap();

    let (restored, _restored_dir) = setup().await?;
    let err = restore_repository(&restored, TENANT, REPO, backup_dir.path())
        .await
        .expect_err("a node without a path is refused");
    assert!(err.to_string().contains("no path"), "{err}");
    assert_eq!(path_at(&restored, "page", None).await, None);
    Ok(())
}

/// Graph relations are not part of the node records: a backup used to restore
/// every node and not a single edge. They now travel in `relations.jsonl`,
/// both directions, type and weight, and NEIGHBORS-style reads (outgoing and
/// incoming) see them after a restore. An UNRELATEd edge is not exported.
#[tokio::test]
async fn backup_round_trip_restores_relations_both_ways() -> Result<()> {
    use crate::node_path_writer_test::{BRANCH, WS};
    use raisin_models::nodes::RelationRef;
    use raisin_storage::{RelationRepository, Storage, StorageScope};

    let (source, _source_dir) = setup().await?;
    repo_create(&source, folder("alice", "/alice")).await?;
    repo_create(&source, folder("bob", "/bob")).await?;
    repo_create(&source, folder("carol", "/carol")).await?;
    let scope = StorageScope::new(TENANT, REPO, BRANCH, WS);
    let rel = |to: &str, ty: &str, w: f32| {
        RelationRef::new(
            to.into(),
            WS.into(),
            "raisin:Folder".into(),
            ty.into(),
            Some(w),
        )
    };
    let rels = source.relations();
    rels.add_relation(
        scope,
        "alice",
        "raisin:Folder",
        rel("bob", "FRIENDS_WITH", 1.0),
    )
    .await?;
    rels.add_relation(
        scope,
        "bob",
        "raisin:Folder",
        rel("alice", "FRIENDS_WITH", 0.5),
    )
    .await?;
    rels.add_relation(scope, "alice", "raisin:Folder", rel("carol", "BLOCKS", 1.0))
        .await?;
    rels.remove_relation(scope, "alice", WS, "carol", Some("BLOCKS"))
        .await?;

    let backup_dir = TempDir::new().unwrap();
    backup_repository(&source, TENANT, REPO, backup_dir.path()).await?;
    let file = backup_dir
        .path()
        .join(TENANT)
        .join(REPO)
        .join("relations.jsonl");
    let text = std::fs::read_to_string(&file).unwrap();
    assert_eq!(text.lines().count(), 2, "{text}");
    assert!(
        !text.contains("BLOCKS"),
        "a removed edge is not exported: {text}"
    );

    let (restored, _restored_dir) = setup().await?;
    restore_repository(&restored, TENANT, REPO, backup_dir.path()).await?;
    let rr = restored.relations();
    let out = rr.get_outgoing_relations(scope, "alice", None).await?;
    assert_eq!(out.len(), 1, "{out:?}");
    assert_eq!(out[0].target, "bob");
    assert_eq!(out[0].relation_type, "FRIENDS_WITH");
    assert_eq!(out[0].weight, Some(1.0));
    let incoming = rr.get_incoming_relations(scope, "alice", None).await?;
    assert_eq!(incoming.len(), 1, "{incoming:?}");
    assert_eq!(incoming[0].0, WS, "the source's workspace");
    assert_eq!(incoming[0].1, "bob");
    assert_eq!(incoming[0].2.weight, Some(0.5));
    let global = rr
        .scan_relations_global(
            raisin_storage::BranchScope::new(TENANT, REPO, BRANCH),
            None,
            None,
        )
        .await?;
    assert_eq!(global.len(), 2, "{global:?}");
    Ok(())
}

/// A backup from a release without `relations.jsonl` still restores.
#[tokio::test]
async fn restore_without_a_relations_file_still_works() -> Result<()> {
    let (source, _source_dir) = setup().await?;
    repo_create(&source, folder("page", "/page")).await?;
    let backup_dir = TempDir::new().unwrap();
    backup_repository(&source, TENANT, REPO, backup_dir.path()).await?;
    std::fs::remove_file(
        backup_dir
            .path()
            .join(TENANT)
            .join(REPO)
            .join("relations.jsonl"),
    )
    .unwrap();
    let (restored, _restored_dir) = setup().await?;
    restore_repository(&restored, TENANT, REPO, backup_dir.path()).await?;
    assert_eq!(
        path_at(&restored, "page", None).await.as_deref(),
        Some("/page")
    );
    Ok(())
}

/// Relations embedded in a node record (`Node.relations`, as a restore or a
/// batch write carries them) are indexed like RELATE indexes them: the reverse
/// entry names the SOURCE workspace, and the global entry exists.
#[tokio::test]
async fn embedded_relations_index_the_source_on_the_reverse_side() -> Result<()> {
    use crate::node_path_writer_test::{BRANCH, WS};
    use raisin_models::nodes::RelationRef;
    use raisin_storage::{BranchScope, RelationRepository, Storage, StorageScope};

    let (storage, _dir) = setup().await?;
    let mut dave = folder("dave", "/dave");
    dave.relations = vec![RelationRef::new(
        "alice".into(),
        "people".into(),
        "raisin:Folder".into(),
        "KNOWS".into(),
        Some(2.0),
    )];
    repo_create(&storage, dave).await?;

    let incoming = storage
        .relations()
        .get_incoming_relations(
            StorageScope::new(TENANT, REPO, BRANCH, "people"),
            "alice",
            None,
        )
        .await?;
    assert_eq!(incoming.len(), 1, "{incoming:?}");
    assert_eq!(incoming[0].0, WS, "the source workspace, not the target's");
    assert_eq!(incoming[0].1, "dave");
    let global = storage
        .relations()
        .scan_relations_global(BranchScope::new(TENANT, REPO, BRANCH), Some("KNOWS"), None)
        .await?;
    assert_eq!(global.len(), 1, "{global:?}");
    Ok(())
}
