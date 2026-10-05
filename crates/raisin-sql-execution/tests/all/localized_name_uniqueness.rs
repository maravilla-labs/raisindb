//! Plan Phase 13c, SQL surface: `UPDATE … FOR LOCALE … SET __node_name`
//! writes through the transaction translation path, and obeys localized-name
//! sibling uniqueness there — against stored siblings and against the other
//! rows of the SAME statement — only when the repository enforces it.

use super::localized_paths::{config, engine, rows, setup_with, Store, B, R, T, WS};
use futures::StreamExt;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_rocksdb::management::async_indexing::repair::{run_repair, RepairKind, RepairOptions};
use raisin_sql_execution::QueryEngine;
use raisin_storage::{CreateNodeOptions, NodeRepository, Storage, StorageScope};
use std::sync::Arc;

/// The chair fixture plus `/products/table`, built clean.
async fn setup(enforce: bool) -> (Arc<Store>, tempfile::TempDir) {
    let mut repository = config();
    repository.localized_names.enforce_unique = enforce;
    let (storage, dir, _) = setup_with(repository).await;
    let table = Node {
        id: uuid::Uuid::new_v4().to_string(),
        name: "table".to_string(),
        path: "/products/table".to_string(),
        parent: Some("products".to_string()),
        node_type: "raisin:Folder".to_string(),
        created_at: Some(chrono::Utc::now()),
        ..Node::default()
    };
    storage
        .nodes()
        .create(
            StorageScope::new(T, R, B, WS),
            table,
            CreateNodeOptions {
                validate_schema: false,
                validate_parent_allows_child: false,
                validate_workspace_allows_type: false,
                operation_meta: None,
            },
        )
        .await
        .unwrap();
    let options = RepairOptions {
        check_headroom: false,
        max_bytes_per_sec: 0,
        ..RepairOptions::default()
    };
    let reports = run_repair(&storage, T, R, Some(B), RepairKind::LocalizedNames, options)
        .await
        .unwrap();
    assert!(reports.iter().all(|r| r.completed), "{reports:?}");
    (storage, dir)
}

/// The statement's error, if any (`execute` or a row).
async fn error_of(engine: &QueryEngine<Store>, sql: &str) -> Option<String> {
    match engine.execute(sql).await {
        Err(e) => Some(e.to_string()),
        Ok(mut stream) => {
            while let Some(row) = stream.next().await {
                if let Err(e) = row {
                    return Some(e.to_string());
                }
            }
            None
        }
    }
}

/// The id `RESOLVE_PATH` answers for a French path.
async fn resolve_fr(engine: &QueryEngine<Store>, path: &str) -> Option<String> {
    let out = rows(
        engine,
        &format!(
            "SELECT RESOLVE_PATH('{WS}', 'fr', '{path}') AS id FROM '{WS}' \
             WHERE path = '/products'"
        ),
    )
    .await;
    out[0].iter().find_map(|(k, v)| match v {
        PropertyValue::String(s) if k.ends_with("id") => Some(s.clone()),
        _ => None,
    })
}

async fn id_of(storage: &Store, path: &str) -> String {
    storage
        .nodes()
        .get_by_path(StorageScope::new(T, R, B, WS), path, None)
        .await
        .unwrap()
        .unwrap()
        .id
}

const NAME_TABLE_CHAISE: &str =
    "UPDATE pages FOR LOCALE 'fr' SET __node_name = '/chaise/' WHERE path = '/products/table'";

#[tokio::test]
async fn update_for_locale_node_name_colliding_with_a_sibling_is_refused_when_enforced() {
    let (storage, _dir) = setup(true).await;
    let sql = engine(&storage);
    let error = error_of(&sql, NAME_TABLE_CHAISE).await;
    assert!(
        error
            .as_deref()
            .is_some_and(|e| e.contains("localized node name 'chaise'")),
        "{error:?}"
    );
    let chair = id_of(&storage, "/products/chair").await;
    assert_eq!(resolve_fr(&sql, "/produits/chaise").await, Some(chair));

    // Every write of ONE transaction is checked against the others (TRANSLATE
    // filters one node per statement, so: two statements, one transaction).
    let tx = engine(&storage); // a second engine: its own transaction context
    assert_eq!(error_of(&tx, "BEGIN").await, None);
    assert_eq!(
        error_of(
            &tx,
            "UPDATE pages FOR LOCALE 'fr' SET __node_name = 'meuble' WHERE path = '/products/chair'",
        )
        .await,
        None
    );
    let error = error_of(
        &tx,
        "UPDATE pages FOR LOCALE 'fr' SET __node_name = 'meuble' WHERE path = '/products/table'",
    )
    .await;
    assert!(
        error
            .as_deref()
            .is_some_and(|e| e.contains("localized node name 'meuble'")),
        "{error:?}"
    );
    assert_eq!(resolve_fr(&sql, "/produits/meuble").await, None);
}

#[tokio::test]
async fn update_for_locale_node_name_colliding_with_a_sibling_is_accepted_when_not_enforced() {
    let (storage, _dir) = setup(false).await;
    let sql = engine(&storage);
    assert_eq!(error_of(&sql, NAME_TABLE_CHAISE).await, None);
    let table = id_of(&storage, "/products/table").await;
    let products = id_of(&storage, "/products").await;
    let claims = raisin_rocksdb::localized_name::rows::claims(
        storage.db(),
        raisin_rocksdb::localized_name::NameScope::new(T, R, B, WS),
        "fr",
        &products,
        "chaise",
        None,
    )
    .unwrap();
    assert!(
        claims.iter().any(|(_, id)| *id == table),
        "the table claims the (normalized) name too: {claims:?}"
    );
}
