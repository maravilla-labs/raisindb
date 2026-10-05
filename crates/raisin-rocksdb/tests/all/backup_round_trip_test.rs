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
