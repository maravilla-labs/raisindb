//! SQL three-valued logic for comparisons, through the virtual `__node_name`
//! column and through a plain JSON extract alike.
//!
//! Regression: `WHERE locale = 'fr' AND __node_name <> name` counted EVERY
//! node, translated or not. `__node_name` was correctly NULL for a node with no
//! translated name (it reads NULL in the projection too); the comparison
//! evaluator turned `NULL <> x` into TRUE, because `<>` was evaluated as
//! `!(a = b)` over an equality that answered FALSE for a NULL operand. Not
//! specific to the column: `properties->>'missing' <> 'x'` matched every row.

use super::localized_paths::{config, engine, rows, setup_with, Store, B, R, T, WS};
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_sql_execution::QueryEngine;
use raisin_storage::{CreateNodeOptions, NodeRepository, Storage, StorageScope};
use std::sync::Arc;

/// `/about`: a node with NO fr translation (no `/__node_name` overlay), next
/// to the fixture's translated `/products` (`produits`) and `/products/chair`
/// (`chaise`).
async fn add_untranslated(storage: &Arc<Store>) {
    let node = Node {
        id: uuid::Uuid::new_v4().to_string(),
        name: "about".to_string(),
        path: "/about".to_string(),
        node_type: "raisin:Folder".to_string(),
        created_at: Some(chrono::Utc::now()),
        ..Node::default()
    };
    storage
        .nodes()
        .create(
            StorageScope::new(T, R, B, WS),
            node,
            CreateNodeOptions {
                validate_schema: false,
                validate_parent_allows_child: false,
                validate_workspace_allows_type: false,
                operation_meta: None,
            },
        )
        .await
        .unwrap();
}

async fn count(engine: &QueryEngine<Store>, predicate: &str) -> i64 {
    let sql = format!("SELECT COUNT(*) AS n FROM '{WS}' WHERE locale = 'fr' AND {predicate}");
    let out = rows(engine, &sql).await;
    match out[0].values().next() {
        Some(PropertyValue::Integer(n)) => *n,
        Some(PropertyValue::Float(n)) => *n as i64,
        other => panic!("not a count [{sql}]: {other:?}"),
    }
}

async fn paths(engine: &QueryEngine<Store>, predicate: &str) -> Vec<String> {
    let sql = format!("SELECT path FROM '{WS}' WHERE locale = 'fr' AND {predicate}");
    let mut out: Vec<String> = rows(engine, &sql)
        .await
        .into_iter()
        .filter_map(|row| match row.into_values().next() {
            Some(PropertyValue::String(s)) => Some(s),
            _ => None,
        })
        .collect();
    out.sort();
    out
}

#[tokio::test]
async fn a_null_node_name_never_compares_true() {
    let (storage, _dir, _chair) = setup_with(config()).await;
    add_untranslated(&storage).await;
    let engine = engine(&storage);

    // The prod query: only the two nodes that HAVE a differing fr name.
    assert_eq!(count(&engine, "__node_name <> name").await, 2);
    assert_eq!(
        paths(&engine, "__node_name <> name").await,
        vec!["/products".to_string(), "/products/chair".to_string()]
    );
    // NULL = NULL is NULL, not TRUE.
    assert_eq!(count(&engine, "__node_name = __node_name").await, 2);
    // NOT of an unknown comparison is still unknown.
    assert_eq!(count(&engine, "NOT (__node_name = name)").await, 2);
    assert_eq!(count(&engine, "NOT (__node_name < 'zzz')").await, 0);
    // IN / NOT IN / BETWEEN over a NULL operand are unknown too.
    assert_eq!(count(&engine, "__node_name NOT IN ('nope')").await, 2);
    assert_eq!(
        count(&engine, "NOT (__node_name BETWEEN 'a' AND 'zzz')").await,
        0
    );
    // The NULL-aware spellings still see the untranslated node.
    assert_eq!(count(&engine, "__node_name IS NULL").await, 1);
    assert_eq!(count(&engine, "__node_name IS DISTINCT FROM name").await, 3);
}

#[tokio::test]
async fn a_missing_json_property_never_compares_true() {
    let (storage, _dir, _chair) = setup_with(config()).await;
    add_untranslated(&storage).await;
    let engine = engine(&storage);
    assert_eq!(
        count(&engine, "properties->>'missing'::String <> 'x'").await,
        0
    );
    assert_eq!(
        count(&engine, "properties->>'missing'::String = 'x'").await,
        0
    );
    assert_eq!(
        count(&engine, "NOT (properties->>'missing'::String = 'x')").await,
        0
    );
    assert_eq!(
        count(&engine, "properties->>'missing'::String NOT IN ('x', 'y')").await,
        0
    );
    // A list containing NULL: `x NOT IN (…, NULL)` is never TRUE.
    assert_eq!(count(&engine, "name NOT IN ('zzz', NULL)").await, 0);
    // …while a match still wins over the NULL item.
    assert_eq!(count(&engine, "NOT (name NOT IN ('about', NULL))").await, 1);
}
