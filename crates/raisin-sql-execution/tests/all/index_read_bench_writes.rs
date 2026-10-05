//! Write-path baseline for the read/index/resolve plan: **H** — update one
//! property of a node with P = 30 properties, with `index.skip_unchanged` off
//! and on (plan Phase 7). Reports the PROPERTY_INDEX entries and bytes ONE
//! update stages, the entries the node has accumulated after all updates, and
//! the median update time. `#[ignore]`:
//!
//! ```bash
//! cargo test -p raisin-sql-execution --test all index_read_bench_h -- --ignored --nocapture
//! ```

use super::index_read_bench::{
    bootstrap, insert_many, jsonb, median_of, record, run, unlock_skip_unchanged, Store, BRANCH,
    REPO, TENANT, WS,
};
use raisin_hlc::HLC;
use raisin_storage::{BranchRepository, Storage};
use serde_json::{json, Value};
use std::time::Instant;

const PROPS: usize = 30;
const UPDATES: usize = 50;

fn props(edit: usize) -> Value {
    let mut map = serde_json::Map::new();
    for i in 0..PROPS - 1 {
        map.insert(format!("p{i:02}"), json!(format!("value-{i}")));
    }
    map.insert("counter".to_string(), json!(edit));
    Value::Object(map)
}

/// `(entries, bytes, property names)` of PROPERTY_INDEX for `id`: all of
/// them, or only those at `at`.
fn property_entries(storage: &Store, id: &str, at: Option<&HLC>) -> (usize, usize, Vec<String>) {
    let prefix = raisin_rocksdb::keys::KeyBuilder::new()
        .push(TENANT)
        .push(REPO)
        .push(BRANCH)
        .push(WS)
        .build_prefix();
    let db = storage.db();
    let cf = db.cf_handle(raisin_rocksdb::cf::PROPERTY_INDEX).unwrap();
    let node_suffix = format!("\0{id}");
    let mut out = (0, 0, Vec::new());
    let iter = db.iterator_cf(
        cf,
        rocksdb::IteratorMode::From(&prefix, rocksdb::Direction::Forward),
    );
    for item in iter {
        let (key, value) = item.unwrap();
        if !key.starts_with(&prefix) {
            break;
        }
        if !key.ends_with(node_suffix.as_bytes()) {
            continue;
        }
        if let Some(at) = at {
            let end = key.len() - node_suffix.len();
            if end < 16 || key[end - 16..end] != at.encode_descending() {
                continue;
            }
        }
        out.0 += 1;
        out.1 += key.len() + value.len();
        // {tag}\0{name}\0…
        let name = key[prefix.len()..]
            .split(|b| *b == 0)
            .nth(1)
            .map(|n| String::from_utf8_lossy(n).into_owned())
            .unwrap_or_default();
        let tomb = if value.as_ref() == b"T" { " (T)" } else { "" };
        out.2.push(format!("{name}{tomb}"));
    }
    out
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "write-path baseline; run with --ignored --nocapture"]
async fn index_read_bench_h_update_one_of_30_properties() {
    for skip in [false, true] {
        let (engine, storage, _dir) = bootstrap().await;
        // Explicit both ways: the flag is ON by default since plan Phase 7b
        // (and `bootstrap` already rebuilt the branch under it).
        storage.nodes_impl().set_index_skip_unchanged(skip);
        if skip {
            unlock_skip_unchanged(&storage).await;
        }
        insert_many(
            &engine,
            &[("target".to_string(), "/target".to_string(), props(0))],
        )
        .await;
        let mut samples = Vec::with_capacity(UPDATES);
        for edit in 1..=UPDATES {
            let sql = format!(
                "UPDATE '{WS}' SET properties = {} WHERE path = '/target'",
                jsonb(&props(edit))
            );
            let start = Instant::now();
            run(&engine, &sql).await;
            samples.push(start.elapsed());
        }
        let head = storage
            .branches()
            .get_branch(TENANT, REPO, BRANCH)
            .await
            .expect("branch")
            .expect("exists")
            .head;
        let (puts, bytes, names) = property_entries(&storage, "target", Some(&head));
        let (total, total_bytes, _) = property_entries(&storage, "target", None);
        record(
            "H_update_one_of_30_properties",
            json!({
                "properties": PROPS,
                "updates": UPDATES,
                "skip_unchanged": skip,
                "entries_per_update": puts,
                "bytes_per_update": bytes,
                "entries_of_one_update": if skip { json!(names) } else { json!(null) },
                "entries_after_all_updates": total,
                "bytes_after_all_updates": total_bytes,
            }),
            median_of(samples),
        );
    }
}
