//! Plan Phase 4 through SQL: index scans and RESOLVE read nodes in batches
//! (`get_many_for_read`), through ONE storage view per statement, with
//! `sql.batched_fetch = false` as the per-row rollback.
//!
//! Reuses the MVCC oracle's environment (`oracle:Volatile` is
//! `versionable: false`).

use crate::mvcc_index_oracle::env::{query, Env, MAIN, PAGE, VOLATILE, WS};
use futures::StreamExt;
use raisin_models::nodes::properties::{PropertyValue, RaisinReference};
use raisin_models::nodes::Node;
use raisin_sql_execution::QueryEngine;
use raisin_storage::{NodeRepository, Storage};
use serde_json::Value;
use std::collections::HashMap;

fn node(id: &str, path: &str, node_type: &str, props: &[(&str, PropertyValue)]) -> Node {
    let name = path.rsplit('/').next().unwrap().to_string();
    Node {
        id: id.to_string(),
        name,
        path: path.to_string(),
        node_type: node_type.to_string(),
        properties: props
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect::<HashMap<_, _>>(),
        workspace: Some(WS.to_string()),
        ..Default::default()
    }
}

fn text(s: &str) -> PropertyValue {
    PropertyValue::String(s.to_string())
}

fn reference(id: &str) -> PropertyValue {
    PropertyValue::Reference(RaisinReference {
        id: id.to_string(),
        workspace: WS.to_string(),
        path: String::new(),
    })
}

/// Write `nodes` in ONE transaction.
async fn put_all(env: &Env, nodes: &[Node]) {
    let tx = env.tx(MAIN).await;
    for n in nodes {
        tx.put_node(WS, n).await.expect("put_node");
    }
    tx.commit().await.expect("commit");
}

async fn rows(engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>, sql: &str) -> Vec<Value> {
    query(engine, sql)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"))
}

/// The column `name` (qualified or not) of a row.
fn col<'a>(row: &'a Value, name: &str) -> &'a Value {
    row.as_object()
        .and_then(|o| {
            o.iter()
                .find(|(k, _)| k.as_str() == name || k.rsplit('.').next() == Some(name))
                .map(|(_, v)| v)
        })
        .unwrap_or(&Value::Null)
}

/// One statement reads its candidates in chunks of 256. A `versionable=false`
/// node rewritten in place BETWEEN two chunks keeps its revision, so the
/// statement's revision bound cannot hide the rewrite; the statement's one
/// storage view does. The per-row rollback path shows the window is real.
#[tokio::test]
async fn statement_snapshot_spans_chunks_with_versionable_false_overwrite() {
    let env = Env::new(None).await;
    let nodes: Vec<Node> = (0..300)
        .map(|i| {
            node(
                &format!("v{i:03}"),
                &format!("/v{i:03}"),
                VOLATILE,
                &[("kind", text("k")), ("title", text("old"))],
            )
        })
        .collect();
    put_all(&env, &nodes).await;

    let sql = format!(
        "SELECT id, properties->>'title' AS title FROM '{WS}' \
         WHERE properties->>'kind'::String = 'k'"
    );
    let plan = format!(
        "{:?}",
        rows(&env.engine(MAIN), &format!("EXPLAIN {sql}")).await
    );
    assert!(plan.contains("PropertyIndexScan"), "{plan}");

    // Which node the SECOND chunk starts with: rows come in index order.
    let order: Vec<String> = rows(&env.engine(MAIN), &sql)
        .await
        .iter()
        .map(|r| col(r, "id").as_str().unwrap().to_string())
        .collect();
    assert_eq!(order.len(), 300);

    for (batched, want) in [(true, "old"), (false, "new")] {
        let victim = &order[if batched { 280 } else { 290 }];
        let engine = env.engine(MAIN).with_batched_fetch(batched);
        let mut stream = engine.execute(&sql).await.expect("execute");
        // The first row pulls the first chunk (256 nodes) through the view.
        let first = stream.next().await.expect("a row").expect("row");
        assert!(!first.columns.is_empty());

        let mut rewritten = nodes.iter().find(|n| &n.id == victim).unwrap().clone();
        rewritten.properties.insert("title".into(), text("new"));
        let before = env.head(MAIN).await;
        put_all(&env, &[rewritten]).await;
        assert_eq!(
            env.head(MAIN).await,
            before,
            "an in-place write mints no revision"
        );

        let mut seen = None;
        while let Some(row) = stream.next().await {
            let row = serde_json::to_value(&row.expect("row").columns).unwrap();
            if col(&row, "id").as_str() == Some(victim.as_str()) {
                seen = col(&row, "title").as_str().map(str::to_string);
            }
        }
        assert_eq!(
            seen.as_deref(),
            Some(want),
            "batched_fetch={batched}: {victim} as read by the running statement"
        );
    }
}

/// DESCENDANT_OF after a subtree move answers with the moved paths — through
/// the repository move and the transaction move — and the old parent has no
/// descendants left at HEAD while it still has them at the pre-move revision.
#[tokio::test]
async fn move_then_descendant_of_returns_new_path() {
    let env = Env::new(None).await;
    put_all(
        &env,
        &[
            node("a", "/a", PAGE, &[]),
            node("x", "/x", PAGE, &[]),
            node("y", "/y", PAGE, &[]),
        ],
    )
    .await;
    put_all(&env, &[node("b", "/a/b", PAGE, &[])]).await;
    put_all(&env, &[node("c", "/a/b/c", PAGE, &[])]).await;
    let before = env.head(MAIN).await;
    let engine = env.engine(MAIN);
    let paths = |rows: Vec<Value>| -> Vec<String> {
        rows.iter()
            .map(|r| col(r, "path").as_str().unwrap_or_default().to_string())
            .collect()
    };
    let under = |p: &str| format!("SELECT path FROM '{WS}' WHERE DESCENDANT_OF('{p}')");

    env.storage
        .nodes()
        .move_node_tree(env.scope(MAIN), "b", "/x/b", None)
        .await
        .expect("repository move");
    assert_eq!(
        paths(rows(&engine, &under("/x")).await),
        vec!["/x/b", "/x/b/c"]
    );
    assert!(paths(rows(&engine, &under("/a")).await).is_empty());
    let at_before = format!("{} AND __revision = '{before}'", under("/a"));
    assert_eq!(
        paths(rows(&engine, &at_before).await),
        vec!["/a/b", "/a/b/c"]
    );

    let tx = env.tx(MAIN).await;
    tx.move_node_tree(WS, "b", "/y/b").await.expect("tx move");
    tx.commit().await.expect("commit");
    assert_eq!(
        paths(rows(&engine, &under("/y")).await),
        vec!["/y/b", "/y/b/c"]
    );
    assert!(paths(rows(&engine, &under("/x")).await).is_empty());
    let c = env.storage.nodes().get(env.scope(MAIN), "c", None).await;
    assert_eq!(c.unwrap().unwrap().path, "/y/b/c");
}

/// The rollback switch: every batched read shape returns exactly the rows the
/// per-row path returns.
#[tokio::test]
async fn batched_fetch_off_returns_the_same_rows() {
    let env = Env::new(None).await;
    put_all(
        &env,
        &[node("hub", "/hub", PAGE, &[("title", text("hub"))])],
    )
    .await;
    let mut nodes = Vec::new();
    for i in 0..40 {
        nodes.push(node(
            &format!("p{i:02}"),
            &format!("/hub/p{i:02}"),
            PAGE,
            &[
                ("title", text(if i % 3 == 0 { "alpha" } else { "beta" })),
                ("hub", reference("hub")),
                ("next", reference(&format!("p{:02}", (i + 1) % 40))),
            ],
        ));
    }
    put_all(&env, &nodes).await;

    let queries = [
        format!("SELECT id, properties FROM '{WS}' WHERE properties->>'title'::String = 'alpha'"),
        format!("SELECT id FROM '{WS}' WHERE properties->>'title'::String = 'beta' LIMIT 1"),
        format!("SELECT id, path FROM '{WS}' WHERE REFERENCES('{WS}:/hub')"),
        format!("SELECT id FROM '{WS}' WHERE REFERENCES('{WS}:/hub') LIMIT 3"),
        format!("SELECT id FROM '{WS}' WHERE CHILD_OF('/hub') AND node_type = '{PAGE}' ORDER BY created_at DESC LIMIT 5"),
        format!("SELECT id, RESOLVE(properties, 2) AS r FROM '{WS}' WHERE CHILD_OF('/hub')"),
        format!("SELECT id, RESOLVE(properties) AS r FROM '{WS}' WHERE properties->>'title'::String = 'alpha' LIMIT 1"),
    ];
    // Each batched scan shape is what the planner picked.
    for (sql, scan) in [
        (&queries[0], "PropertyIndexScan"),
        (&queries[2], "ReferenceIndexScan"),
        (&queries[4], "CompoundIndexScan"),
    ] {
        let plan = format!(
            "{:?}",
            rows(&env.engine(MAIN), &format!("EXPLAIN {sql}")).await
        );
        assert!(plan.contains(scan), "{sql}: expected {scan} in {plan}");
    }
    for sql in &queries {
        let on = rows(&env.engine(MAIN), sql).await;
        let off = rows(&env.engine(MAIN).with_batched_fetch(false), sql).await;
        assert!(!on.is_empty(), "{sql} returned rows");
        assert_eq!(on, off, "{sql}");
    }
    // RESOLVE really inlined (both depths), batched.
    let r = rows(&env.engine(MAIN), &queries[5]).await;
    let p01 = r.iter().find(|row| col(row, "id") == "p01").expect("p01");
    assert_eq!(col(p01, "r")["hub"]["id"], "hub");
    assert_eq!(col(p01, "r")["next"]["next"]["id"], "p03");
}

/// A property literally named `properties` must not replace the `properties`
/// column under `SELECT *` — and the borrowed conversion (a path lookup) must
/// answer like the owned one (an index scan), whichever scan the planner picks.
#[tokio::test]
async fn property_named_properties_does_not_replace_the_map_column() {
    let env = Env::new(None).await;
    put_all(
        &env,
        &[node(
            "odd",
            "/odd",
            PAGE,
            &[("kind", text("odd")), ("properties", text("shadow"))],
        )],
    )
    .await;
    let by_path = format!("SELECT * FROM '{WS}' WHERE path = '/odd'");
    let by_index = format!("SELECT * FROM '{WS}' WHERE properties->>'kind'::String = 'odd'");
    let plan = format!(
        "{:?}",
        rows(&env.engine(MAIN), &format!("EXPLAIN {by_index}")).await
    );
    assert!(plan.contains("PropertyIndexScan"), "{plan}");

    let mut seen = Vec::new();
    for sql in [&by_path, &by_index] {
        let got = rows(&env.engine(MAIN), sql).await;
        assert_eq!(got.len(), 1, "{sql}");
        let map = col(&got[0], "properties");
        assert_eq!(map["kind"], "odd", "{sql}: the column is the map: {map}");
        assert_eq!(map["properties"], "shadow", "{sql}");
        seen.push(map.clone());
    }
    assert_eq!(seen[0], seen[1], "both conversions agree");
}
