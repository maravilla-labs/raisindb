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
        [cf::TRANSLATION_DATA, cf::TRANSLATION_INDEX],
    )
    .unwrap();
    (Arc::new(db), dir)
}

/// A realistic revision: its descending encoding starts with `0xFF`, which is
/// not valid UTF-8 — exactly what the old decode choked on.
fn rev(offset: u64) -> HLC {
    HLC::new(1_705_843_009_213 + offset, 0)
}

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
        put_data(&db, "n1", locale, &rev(i as u64), b"{}");
        // A second, newer version of each: still one entry per locale.
        put_data(&db, "n1", locale, &rev(10 + i as u64), b"{}");
    }
    // Another node's translation must not leak in.
    put_data(&db, "n10", "fr", &rev(0), b"{}");

    assert_eq!(list(&db, "n1").await, vec!["de", "de-CH", "en"]);
}

#[tokio::test]
async fn a_locale_is_listed_only_when_its_newest_version_is_live() {
    let (db, _dir) = open();
    // fr: live, then deleted — gone.
    put_data(&db, "n1", "fr", &rev(1), b"{}");
    put_data(&db, "n1", "fr", &rev(2), b"T");
    // en: deleted, then re-translated — listed.
    put_data(&db, "n1", "en", &rev(1), b"T");
    put_data(&db, "n1", "en", &rev(2), b"{}");

    assert_eq!(list(&db, "n1").await, vec!["en"]);
}

#[tokio::test]
async fn every_node_with_a_locale_is_listed() {
    let (db, _dir) = open();
    let cf_index = db.cf_handle(cf::TRANSLATION_INDEX).unwrap();
    for (i, node) in ["n1", "n2", "n3"].into_iter().enumerate() {
        let key = keys::translation_index_key("t", "r", "de-CH", &rev(i as u64), node);
        db.put_cf(cf_index, key, b"").unwrap();
    }
    // A revision whose encoding contains `\0` bytes (counter u64::MAX).
    let key = keys::translation_index_key("t", "r", "de-CH", &HLC::new(7, u64::MAX), "n4");
    db.put_cf(cf_index, key, b"").unwrap();

    let locale = LocaleCode::parse("de-CH").unwrap();
    let mut nodes = queries::list_nodes_with_translation(&db, "t", "r", &locale, &rev(1_000))
        .await
        .unwrap();
    nodes.sort();
    assert_eq!(nodes, vec!["n1", "n2", "n3", "n4"]);
}
