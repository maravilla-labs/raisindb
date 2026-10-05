//! Scenario C of the read bench: RESOLVE (plan Phases 3 and 4).
//!
//! ```bash
//! cargo test -p raisin-sql-execution --test all index_read_bench_c -- --ignored --nocapture
//! RAISIN_SQL_BATCHED_FETCH=0 cargo test -p raisin-sql-execution --test all index_read_bench_c -- --ignored --nocapture
//! ```
//!
//! The second run takes the per-row rollback path (`sql.batched_fetch =
//! false`); each JSON line carries `batched_fetch`, so the two diff cleanly.

use super::index_read_bench::{bootstrap, insert_many, median, record, WS};
use serde_json::{json, Value};

fn reference(id: &str) -> Value {
    json!({ "raisin:ref": id, "raisin:workspace": WS })
}

/// Scenario C: RESOLVE over a site "chrome" node.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "read-path baseline; run with --ignored --nocapture"]
async fn index_read_bench_c_resolve_chrome() {
    let (engine, _storage, _dir) = bootstrap().await;

    // 20 distinct link targets + 3 shared ones (logo, legal, brand).
    let mut rows: Vec<(String, String, Value)> = (0..20)
        .map(|i| {
            (
                format!("a{i}"),
                format!("/a{i}"),
                json!({ "title": format!("Link {i}"), "url": format!("/l/{i}"), "alt": "x" }),
            )
        })
        .collect();
    for shared in ["logo", "legal", "brand"] {
        rows.push((
            shared.to_string(),
            format!("/{shared}"),
            json!({ "title": shared, "file": format!("/{shared}.svg") }),
        ));
    }
    insert_many(&engine, &rows).await;

    // 25 references, 23 distinct targets; logo appears three times.
    let links = |from: usize, to: usize| -> Vec<Value> {
        (from..to).map(|i| reference(&format!("a{i}"))).collect()
    };
    let chrome = json!({
        "header": { "logo": reference("logo"), "nav": links(0, 8) },
        "footer": { "logo": reference("logo"), "legal": reference("legal"), "links": links(8, 16) },
        "settings": { "brand": reference("brand"), "favicon": reference("logo"), "social": links(16, 20) },
    });
    insert_many(&engine, &[("chrome".into(), "/chrome".into(), chrome)]).await;

    let one = format!("SELECT RESOLVE(properties, 2) AS r FROM '{WS}' WHERE path = '/chrome'");
    let took = median(&engine, &one, 1).await;
    record(
        "C_resolve_chrome_one_row",
        json!({ "refs": 25, "distinct": 23, "batched_fetch": batched_fetch_env() }),
        took,
    );

    // 50 pages that each reference the chrome: the targets are shared by
    // every row of the statement.
    insert_many(&engine, &[("pages".into(), "/pages".into(), json!({}))]).await;
    let pages: Vec<(String, String, Value)> = (0..50)
        .map(|i| {
            (
                format!("p{i}"),
                format!("/pages/p{i}"),
                json!({ "title": format!("Page {i}"), "chrome": reference("chrome") }),
            )
        })
        .collect();
    insert_many(&engine, &pages).await;

    let fifty = format!("SELECT RESOLVE(properties, 3) AS r FROM '{WS}' WHERE CHILD_OF('/pages')");
    let took = median(&engine, &fifty, 50).await;
    record(
        "C_resolve_chrome_50_rows",
        json!({ "rows": 50, "depth": 3, "batched_fetch": batched_fetch_env() }),
        took,
    );

    // C3: a listing whose rows share NOTHING — 50 articles, each with its own
    // author and image. The memo cannot help here; only reading a chunk of
    // rows' targets in one batch per level can (plan Phase 4).
    let mut targets: Vec<(String, String, Value)> = Vec::new();
    for i in 0..50 {
        targets.push((
            format!("au{i}"),
            format!("/au{i}"),
            json!({ "name": format!("Author {i}") }),
        ));
        targets.push((
            format!("im{i}"),
            format!("/im{i}"),
            json!({ "file": format!("/im{i}.jpg") }),
        ));
    }
    insert_many(&engine, &targets).await;
    insert_many(
        &engine,
        &[("articles".into(), "/articles".into(), json!({}))],
    )
    .await;
    let articles: Vec<(String, String, Value)> = (0..50)
        .map(|i| {
            (
                format!("ar{i}"),
                format!("/articles/ar{i}"),
                json!({
                    "title": format!("Article {i}"),
                    "author": reference(&format!("au{i}")),
                    "image": reference(&format!("im{i}")),
                }),
            )
        })
        .collect();
    insert_many(&engine, &articles).await;
    let listing =
        format!("SELECT RESOLVE(properties, 1) AS r FROM '{WS}' WHERE CHILD_OF('/articles')");
    let took = median(&engine, &listing, 50).await;
    record(
        "C3_resolve_listing_50_rows_distinct_targets",
        json!({ "rows": 50, "distinct": 100, "batched_fetch": batched_fetch_env() }),
        took,
    );
}

/// `sql.batched_fetch` as this process runs it (`RAISIN_SQL_BATCHED_FETCH`),
/// so a before/after pair of runs can be told apart in the JSON lines.
fn batched_fetch_env() -> bool {
    std::env::var("RAISIN_SQL_BATCHED_FETCH")
        .map(|v| {
            !matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "off" | "no"
            )
        })
        .unwrap_or(true)
}
