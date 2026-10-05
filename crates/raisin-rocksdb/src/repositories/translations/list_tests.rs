//! Translation listings must survive the binary revision in their keys.
//!
//! Both listings used to `from_utf8` the whole key remainder, which includes
//! the 16-byte descending revision. That is almost never valid UTF-8 (a real
//! timestamp's top byte negates to `0xFF`), so the decode failed and the entry
//! was dropped without a word: a node with three translations listed none.

use super::{keys, nodes, queries};
use crate::cf;
use raisin_hlc::HLC;
use raisin_models::translations::LocaleCode;
use rocksdb::{Options, DB};
use std::sync::Arc;

fn open() -> (Arc<DB>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let mut opts = Options::default();
    opts.create_if_missing(true);
    opts.create_missing_column_families(true);
    let db = DB::open_cf(
        &opts,
        dir.path(),
        [
            cf::TRANSLATION_DATA,
            cf::TRANSLATION_INDEX,
            cf::INDEX_STATUS,
            cf::NODES,
        ],
    )
    .unwrap();
    (Arc::new(db), dir)
}

/// A realistic revision: its descending encoding starts with `0xFF`, which is
/// not valid UTF-8 — exactly what the old decode choked on.
fn rev(offset: u64) -> HLC {
    HLC::new(1_705_843_009_213 + offset, 0)
}

/// A stored live overlay (the listing decodes the version that decides).
const LIVE: &[u8] = br#"{"type":"properties","data":{}}"#;

fn put_data(db: &DB, node_id: &str, locale: &str, revision: &HLC, value: &[u8]) {
    let key = keys::translation_key("t", "r", "main", "ws", node_id, locale, revision);
    let cf = db.cf_handle(cf::TRANSLATION_DATA).unwrap();
    db.put_cf(cf, key, value).unwrap();
}

async fn list(db: &Arc<DB>, node_id: &str) -> Vec<String> {
    let mut locales: Vec<String> =
        nodes::list_translations_for_node(db, "t", "r", "main", "ws", node_id, &rev(1_000))
            .await
            .unwrap()
            .iter()
            .map(|l| l.as_str().to_string())
            .collect();
    locales.sort();
    locales
}

#[tokio::test]
async fn every_locale_of_a_node_is_listed() {
    let (db, _dir) = open();
    assert!(
        rev(0).encode_descending()[0] == 0xFF,
        "fixture must use a revision whose encoding is not UTF-8"
    );

    for (i, locale) in ["de-CH", "de", "en"].into_iter().enumerate() {
        put_data(&db, "n1", locale, &rev(i as u64), LIVE);
        // A second, newer version of each: still one entry per locale.
        put_data(&db, "n1", locale, &rev(10 + i as u64), LIVE);
    }
    // Another node's translation must not leak in.
    put_data(&db, "n10", "fr", &rev(0), LIVE);

    assert_eq!(list(&db, "n1").await, vec!["de", "de-CH", "en"]);
}

#[tokio::test]
async fn a_locale_is_listed_only_when_its_newest_version_is_live() {
    let (db, _dir) = open();
    // fr: live, then deleted — gone.
    put_data(&db, "n1", "fr", &rev(1), LIVE);
    put_data(&db, "n1", "fr", &rev(2), b"T");
    // en: deleted, then re-translated — listed.
    put_data(&db, "n1", "en", &rev(1), b"T");
    put_data(&db, "n1", "en", &rev(2), LIVE);

    assert_eq!(list(&db, "n1").await, vec!["en"]);
}

#[tokio::test]
async fn every_node_with_a_locale_is_listed() {
    let (db, _dir) = open();
    let cf_index = db.cf_handle(cf::TRANSLATION_INDEX).unwrap();
    for (i, node) in ["n1", "n2", "n3"].into_iter().enumerate() {
        let key = keys::translation_index_key("t", "r", "de-CH", &rev(i as u64), node);
        db.put_cf(cf_index, key, b"").unwrap();
        put_data(&db, node, "de-CH", &rev(i as u64), LIVE);
    }
    // A revision whose encoding contains `\0` bytes (counter u64::MAX).
    let at = HLC::new(7, u64::MAX);
    let key = keys::translation_index_key("t", "r", "de-CH", &at, "n4");
    db.put_cf(cf_index, key, b"").unwrap();
    put_data(&db, "n4", "de-CH", &at, LIVE);

    let locale = LocaleCode::parse("de-CH").unwrap();
    let mut nodes =
        queries::list_nodes_with_translation(&db, "t", "r", "main", "ws", &locale, &rev(1_000))
            .await
            .unwrap();
    nodes.sort();
    assert_eq!(nodes, vec!["n1", "n2", "n3", "n4"]);
}

/// `TRANSLATION_INDEX` is repo-wide (no branch in its key). A node delete on
/// a feature branch writes a `T` entry there; letting the newest entry decide
/// hid the node from `main`'s listing although `main` still serves its
/// overlay — and a translation written only on the feature branch listed the
/// node on `main`.
#[tokio::test]
async fn a_delete_on_another_branch_does_not_hide_the_node() {
    let (db, _dir) = open();
    let cf_index = db.cf_handle(cf::TRANSLATION_INDEX).unwrap();
    let put_on = |branch: &str, node: &str, revision: &HLC, value: &[u8]| {
        let key = keys::translation_key("t", "r", branch, "ws", node, "fr", revision);
        db.put_cf(db.cf_handle(cf::TRANSLATION_DATA).unwrap(), key, value)
            .unwrap();
        let index = keys::translation_index_key("t", "r", "fr", revision, node);
        db.put_cf(cf_index, index, value).unwrap();
    };
    // n1: live on main at r1; deleted on `feature` (forked) at r2.
    put_on("main", "n1", &rev(1), LIVE);
    put_on("feature", "n1", &rev(2), b"T");
    // n2: translated only on `feature`.
    put_on("feature", "n2", &rev(3), LIVE);

    let locale = LocaleCode::parse("fr").unwrap();
    let on = |branch: &'static str| {
        let db = db.clone();
        let locale = locale.clone();
        async move {
            let mut nodes =
                queries::list_nodes_with_translation(&db, "t", "r", branch, "ws", &locale, &rev(9))
                    .await
                    .unwrap();
            nodes.sort();
            nodes
        }
    };
    assert_eq!(on("main").await, vec!["n1"]);
    assert_eq!(on("feature").await, vec!["n2"]);
}
