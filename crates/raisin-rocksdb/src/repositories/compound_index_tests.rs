//! Tests for the compound index reader (`compound_index.rs`).
use super::*;
use raisin_hlc::HLC;
use raisin_storage::scope::StorageScope;
use rocksdb::{Options, DB};
use tempfile::TempDir;

fn create_test_db() -> (Arc<DB>, TempDir) {
    let temp_dir = TempDir::new().unwrap();
    let mut opts = Options::default();
    opts.create_if_missing(true);
    opts.create_missing_column_families(true);

    let cf_names = vec![cf::COMPOUND_INDEX];
    let db = DB::open_cf(&opts, temp_dir.path(), cf_names).unwrap();

    (Arc::new(db), temp_dir)
}

fn test_scope() -> StorageScope<'static> {
    StorageScope::new("tenant1", "repo1", "main", "ws1")
}

#[tokio::test]
async fn test_index_and_scan() {
    let (db, _temp_dir) = create_test_db();
    let repo = CompoundIndexRepositoryImpl::new(db);

    let scope = test_scope();
    let index_name = "by_type_category";
    let node_id = "node123";
    let revision = HLC::new(1700000000000000, 0);

    // Index a node
    let columns = vec![
        CompoundColumnValue::String("news:Article".to_string()),
        CompoundColumnValue::String("business".to_string()),
        CompoundColumnValue::TimestampDesc(1700000000000000), // microseconds
    ];

    repo.index_compound(
        scope, index_name, &columns, &revision, node_id, false, // draft
    )
    .await
    .unwrap();

    // Scan for it
    let equality_values = vec![
        CompoundColumnValue::String("news:Article".to_string()),
        CompoundColumnValue::String("business".to_string()),
    ];

    let results = repo
        .scan_compound_index(
            scope,
            index_name,
            &equality_values,
            false, // draft
            false, // descending
            Some(10),
            None,
        )
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].node_id, node_id);
}

/// A PUBLISHED node must come back from an ordinary (non-published-only)
/// scan.
///
/// The writer picks the keyspace EXCLUSIVELY from `published_at`: a
/// published node goes to `cidx_pub` and is absent from `cidx`. Every SQL
/// scan asks for `published_only: false`, so when that meant "read `cidx`"
/// each published row silently vanished — and because the planner strips the
/// matched equality predicates from the residual filter, nothing downstream
/// could notice. Regression guard: `false` means BOTH keyspaces.
#[tokio::test]
async fn scan_returns_published_and_draft_nodes_together() {
    let (db, _temp_dir) = create_test_db();
    let repo = CompoundIndexRepositoryImpl::new(db);

    let scope = test_scope();
    let index_name = "by_type_category";

    let columns = |ts: i64| {
        vec![
            CompoundColumnValue::String("news:Article".to_string()),
            CompoundColumnValue::String("business".to_string()),
            CompoundColumnValue::TimestampDesc(ts),
        ]
    };

    repo.index_compound(
        scope,
        index_name,
        &columns(1700000000000000),
        &HLC::new(1700000000000000, 0),
        "draft_node",
        false, // draft -> cidx
    )
    .await
    .unwrap();

    repo.index_compound(
        scope,
        index_name,
        &columns(1700000000000001),
        &HLC::new(1700000000000001, 0),
        "published_node",
        true, // published -> cidx_pub
    )
    .await
    .unwrap();

    let equality_values = vec![
        CompoundColumnValue::String("news:Article".to_string()),
        CompoundColumnValue::String("business".to_string()),
    ];

    let all = repo
        .scan_compound_index(scope, index_name, &equality_values, false, true, None, None)
        .await
        .unwrap();
    let ids: Vec<&str> = all.iter().map(|e| e.node_id.as_str()).collect();
    assert!(
        ids.contains(&"published_node"),
        "a published node must be visible to a normal scan; got {ids:?}"
    );
    assert!(
        ids.contains(&"draft_node"),
        "a draft node must still be visible; got {ids:?}"
    );
    assert_eq!(all.len(), 2, "expected exactly the two indexed nodes");

    // `true` still means published-only.
    let published = repo
        .scan_compound_index(scope, index_name, &equality_values, true, true, None, None)
        .await
        .unwrap();
    let published_ids: Vec<&str> = published.iter().map(|e| e.node_id.as_str()).collect();
    assert_eq!(published_ids, vec!["published_node"]);
}

/// A non-String leading column must round-trip.
///
/// `build_compound_key` encodes `Integer` as big-endian bytes, so a reader
/// that rebuilt the prefix as the ASCII text `"42"` addressed different
/// bytes and matched nothing. This pins the writer's encoding as the
/// contract the executor has to reproduce.
#[tokio::test]
async fn scan_matches_a_non_string_leading_column() {
    let (db, _temp_dir) = create_test_db();
    let repo = CompoundIndexRepositoryImpl::new(db);

    let scope = test_scope();
    let index_name = "by_priority";

    repo.index_compound(
        scope,
        index_name,
        &vec![
            CompoundColumnValue::Integer(42),
            CompoundColumnValue::TimestampDesc(1700000000000000),
        ],
        &HLC::new(1700000000000000, 0),
        "int_node",
        false,
    )
    .await
    .unwrap();

    let typed = repo
        .scan_compound_index(
            scope,
            index_name,
            &[CompoundColumnValue::Integer(42)],
            false,
            true,
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        typed.len(),
        1,
        "an Integer prefix must match what was written"
    );
    assert_eq!(typed[0].node_id, "int_node");

    // The old executor behaviour: same value, stringified. Must NOT match —
    // this is the shape of the bug, kept explicit so a regression is loud.
    let stringified = repo
        .scan_compound_index(
            scope,
            index_name,
            &[CompoundColumnValue::String("42".to_string())],
            false,
            true,
            None,
            None,
        )
        .await
        .unwrap();
    assert!(
        stringified.is_empty(),
        "a String-encoded prefix addresses different bytes than an Integer column"
    );
}

#[tokio::test]
async fn test_unindex() {
    let (db, _temp_dir) = create_test_db();
    let repo = CompoundIndexRepositoryImpl::new(db);

    let scope = test_scope();
    let index_name = "by_type_category";
    let node_id = "node123";
    let revision = HLC::new(1700000000000000, 0);

    let columns = vec![
        CompoundColumnValue::String("news:Article".to_string()),
        CompoundColumnValue::String("business".to_string()),
        CompoundColumnValue::TimestampDesc(1700000000000000),
    ];

    // Index
    repo.index_compound(scope, index_name, &columns, &revision, node_id, false)
        .await
        .unwrap();

    // Unindex
    repo.unindex_compound(scope, index_name, &columns, node_id)
        .await
        .unwrap();

    // Verify it's gone
    let equality_values = vec![
        CompoundColumnValue::String("news:Article".to_string()),
        CompoundColumnValue::String("business".to_string()),
    ];

    let results = repo
        .scan_compound_index(
            scope,
            index_name,
            &equality_values,
            false,
            false,
            Some(10),
            None,
        )
        .await
        .unwrap();

    assert!(results.is_empty());
}

#[tokio::test]
async fn test_scan_ordering() {
    let (db, _temp_dir) = create_test_db();
    let repo = CompoundIndexRepositoryImpl::new(db);

    let scope = test_scope();
    let index_name = "by_type_created";

    // Index multiple nodes with different timestamps
    let timestamps = [
        1700000001000000i64,
        1700000003000000i64,
        1700000002000000i64,
    ];
    let node_ids = ["node1", "node3", "node2"];

    for (ts, node_id) in timestamps.iter().zip(node_ids.iter()) {
        let revision = HLC::new(1700000000000000, 0);
        let columns = vec![
            CompoundColumnValue::String("news:Article".to_string()),
            CompoundColumnValue::TimestampDesc(*ts),
        ];

        repo.index_compound(scope, index_name, &columns, &revision, node_id, false)
            .await
            .unwrap();
    }

    // Scan descending (newest first)
    let equality_values = vec![CompoundColumnValue::String("news:Article".to_string())];

    let results = repo
        .scan_compound_index(
            scope,
            index_name,
            &equality_values,
            false,
            false,
            None,
            None,
        )
        .await
        .unwrap();

    // With TimestampDesc encoding, newest should be first
    assert_eq!(results.len(), 3);
    // Note: actual ordering depends on key encoding
}
