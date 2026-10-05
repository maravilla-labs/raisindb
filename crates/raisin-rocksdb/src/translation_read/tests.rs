//! Newest-wins over `TRANSLATION_DATA`, for one locale and for the listing.

use super::{live_locales, locale_prefix, read_overlay, read_version};
use crate::cf;
use raisin_hlc::HLC;
use raisin_models::translations::LocaleOverlay;
use rocksdb::{Options, DB};

fn open() -> (DB, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let mut opts = Options::default();
    opts.create_if_missing(true);
    opts.create_missing_column_families(true);
    (
        DB::open_cf(
            &opts,
            dir.path(),
            [cf::TRANSLATION_DATA, cf::BLOCK_TRANSLATIONS, cf::NODES],
        )
        .unwrap(),
        dir,
    )
}

/// Realistic revisions: their encodings are not valid UTF-8.
fn rev(offset: u64) -> HLC {
    HLC::new(1_705_843_009_213 + offset, 0)
}

fn put(db: &DB, locale: &str, revision: &HLC, value: &[u8]) {
    let mut key = locale_prefix("t", "r", "main", "ws", "n1", locale);
    key.extend_from_slice(&revision.encode_descending());
    db.put_cf(db.cf_handle(cf::TRANSLATION_DATA).unwrap(), key, value)
        .unwrap();
}

fn live(db: &DB) -> Vec<String> {
    let mut locales = live_locales(db, "t", "r", "main", "ws", "n1", None).unwrap();
    locales.sort();
    locales
}

fn overlay(db: &DB, locale: &str, max: Option<&HLC>) -> Option<LocaleOverlay> {
    read_overlay(db, "t", "r", "main", "ws", "n1", locale, max).unwrap()
}

fn hidden() -> Vec<u8> {
    serde_json::to_vec(&LocaleOverlay::Hidden).unwrap()
}

/// A newest tombstone means deleted. The transaction reader used to skip it
/// and return the OLDER live version — resurrecting the deleted translation.
#[test]
fn a_newest_tombstone_hides_every_older_version() {
    let (db, _dir) = open();
    put(&db, "fr", &rev(1), &hidden());
    put(&db, "fr", &rev(2), b"T");

    assert!(overlay(&db, "fr", None).is_none());
    assert!(live(&db).is_empty());

    // The tombstone is still a version that was read (for conflict tracking).
    let version = read_version(&db, "t", "r", "main", "ws", "n1", "fr", None)
        .unwrap()
        .expect("the tombstone is the newest version");
    assert!(version.overlay.is_none());
    assert!(version.key.ends_with(&rev(2).encode_descending()));

    // Before the delete, the translation was there.
    assert!(overlay(&db, "fr", Some(&rev(1))).is_some());
    let at_rev1 = live_locales(&db, "t", "r", "main", "ws", "n1", Some(&rev(1))).unwrap();
    assert_eq!(at_rev1, vec!["fr"]);
}

#[test]
fn a_retranslation_after_a_delete_is_live() {
    let (db, _dir) = open();
    put(&db, "en", &rev(1), &hidden());
    put(&db, "en", &rev(2), b"T");
    put(&db, "en", &rev(3), &hidden());

    assert!(overlay(&db, "en", None).is_some());
    assert!(overlay(&db, "en", Some(&rev(2))).is_none());
    assert_eq!(live(&db), vec!["en"]);
}

#[test]
fn every_live_locale_is_listed_once() {
    let (db, _dir) = open();
    for (i, locale) in ["de-CH", "de", "en"].into_iter().enumerate() {
        put(&db, locale, &rev(i as u64), &hidden());
        put(&db, locale, &rev(10 + i as u64), &hidden());
    }
    put(&db, "fr", &rev(5), b"T");

    assert_eq!(live(&db), vec!["de", "de-CH", "en"]);
    assert!(overlay(&db, "absent", None).is_none());
}

fn node_record_at(db: &DB, revision: &HLC, value: &[u8]) {
    let mut key = crate::keys::node_key_prefix("t", "r", "main", "ws", "n1");
    key.extend_from_slice(&revision.encode_descending());
    db.put_cf(db.cf_handle(cf::NODES).unwrap(), key, value)
        .unwrap();
}

fn delete_node_at(db: &DB, revision: &HLC) {
    node_record_at(db, revision, crate::keys::TOMBSTONE_VALUE);
}

/// A live `NODES` record (the bytes are never decoded by the rule).
fn write_node_at(db: &DB, revision: &HLC) {
    node_record_at(db, revision, b"live");
}

fn put_block(db: &DB, block: &str, locale: &str, revision: &HLC) {
    let mut key = crate::repositories::translations::keys::block_translation_prefix(
        "t", "r", "main", "ws", "n1", block, locale,
    );
    key.extend_from_slice(&revision.encode_descending());
    db.put_cf(db.cf_handle(cf::BLOCK_TRANSLATIONS).unwrap(), key, hidden())
        .unwrap();
}

/// A node delete at `Rd` ends every node-overlay version at or below it, for
/// reads at or after `Rd` — whatever order the two were stored in, and with
/// no translation tombstone stored (the read rule).
#[test]
fn a_node_delete_ends_the_versions_below_it_from_its_revision_on() {
    let (db, _dir) = open();
    write_node_at(&db, &rev(0));
    put(&db, "fr", &rev(1), &hidden());
    put(&db, "de", &rev(5), &hidden());
    delete_node_at(&db, &rev(3));
    write_node_at(&db, &rev(4));

    // Below the delete, fr is there; from it on, it is gone.
    assert!(overlay(&db, "fr", Some(&rev(2))).is_some());
    assert!(overlay(&db, "fr", Some(&rev(3))).is_none());
    assert!(overlay(&db, "fr", None).is_none());
    // de was written after the node was recreated: live.
    assert!(overlay(&db, "de", None).is_some());
    assert_eq!(live(&db), vec!["de"]);
    let at_2 = live_locales(&db, "t", "r", "main", "ws", "n1", Some(&rev(2))).unwrap();
    assert_eq!(at_2, vec!["fr"]);
    // The stored half still sees fr: history GC materializes from it.
    let stored = super::stored_live_locales(&db, "t", "r", "main", "ws", "n1", Some(&rev(3)));
    assert_eq!(stored.unwrap(), vec![("fr".to_string(), rev(1))]);
}

/// Plan Phase 11c: a node delete ends its BLOCK overlays by the same rule —
/// they used to stay live forever, and a recreated node got them back.
#[test]
fn a_node_delete_ends_its_block_overlays_too() {
    let (db, _dir) = open();
    write_node_at(&db, &rev(0));
    put_block(&db, "b1", "fr", &rev(1));
    put_block(&db, "b2", "de", &rev(5));
    delete_node_at(&db, &rev(3));
    write_node_at(&db, &rev(4));

    let block = |name: &str, locale: &str, max: Option<&HLC>| {
        super::read_block_version(&db, "t", "r", "main", "ws", "n1", name, locale, max)
            .unwrap()
            .and_then(|version| version.overlay)
    };
    assert!(block("b1", "fr", Some(&rev(2))).is_some());
    assert!(block("b1", "fr", Some(&rev(3))).is_none());
    assert!(block("b1", "fr", None).is_none());
    // b2/de was written after the node was recreated: live.
    assert!(block("b2", "de", None).is_some());

    let listed = |max: Option<&HLC>| {
        super::live_block_overlays(&db, "t", "r", "main", "ws", "n1", max).unwrap()
    };
    assert_eq!(listed(None), vec![("b2".to_string(), "de".to_string())]);
    assert_eq!(
        listed(Some(&rev(2))),
        vec![("b1".to_string(), "fr".to_string())]
    );
    assert!(listed(Some(&rev(3))).is_empty());
    // The stored half still sees b1/fr: the materializations start from it.
    let stored =
        super::stored_live_block_overlays(&db, ("t", "r", "main", "ws"), "n1", Some(&rev(3)));
    assert_eq!(
        stored.unwrap(),
        vec![("b1".to_string(), "fr".to_string(), rev(1))]
    );
}

/// A version written ABOVE a delete while the node was still deleted (a peer
/// that had not seen the delete, a merge replaying a branch's translation)
/// is ended too — at HEAD, in listings, and after a recreate under the same
/// id. Only "a delete in [R, bound]" used to end a version, so it stayed live
/// for good.
#[test]
fn a_version_written_into_a_dead_generation_stays_ended_after_a_recreate() {
    let (db, _dir) = open();
    write_node_at(&db, &rev(0));
    delete_node_at(&db, &rev(2));
    put(&db, "fr", &rev(3), &hidden());
    put_block(&db, "b1", "fr", &rev(3));
    write_node_at(&db, &rev(5)); // recreated above it

    for bound in [Some(rev(3)), Some(rev(4)), Some(rev(5)), None] {
        let bound = bound.as_ref();
        assert!(overlay(&db, "fr", bound).is_none(), "{bound:?}");
        let block = super::read_block_version(&db, "t", "r", "main", "ws", "n1", "b1", "fr", bound)
            .unwrap()
            .and_then(|version| version.overlay);
        assert!(block.is_none(), "{bound:?}");
        assert!(live_locales(&db, "t", "r", "main", "ws", "n1", bound)
            .unwrap()
            .is_empty());
        assert!(
            super::live_block_overlays(&db, "t", "r", "main", "ws", "n1", bound)
                .unwrap()
                .is_empty()
        );
    }
    // Written in the new generation: live.
    put(&db, "fr", &rev(6), &hidden());
    put_block(&db, "b1", "fr", &rev(6));
    assert!(overlay(&db, "fr", None).is_some());
    assert_eq!(
        super::live_block_overlays(&db, "t", "r", "main", "ws", "n1", None).unwrap(),
        vec![("b1".to_string(), "fr".to_string())]
    );
    // A version with no node record below it at all (its create has not
    // arrived yet) is not ended.
    let (db, _dir) = open();
    put(&db, "fr", &rev(1), &hidden());
    assert!(overlay(&db, "fr", None).is_some());
}

/// Reading every block of a node — the resolver, the copy collectors —
/// walked the node's whole `NODES` history once per `(block, locale)`.
#[test]
fn reading_every_block_of_a_node_walks_its_history_once() {
    let (db, _dir) = open();
    write_node_at(&db, &rev(0));
    for i in 0..40 {
        put_block(&db, &format!("b{i}"), "fr", &rev(1));
        put_block(&db, &format!("b{i}"), "de", &rev(1));
    }
    for i in 2..302 {
        write_node_at(&db, &rev(i));
    }
    let walks = || crate::mvcc_read::WALKS.with(|w| w.get());
    let before = walks();
    let read = super::live_block_versions(&db, ("t", "r", "main", "ws"), "n1", None, |locale| {
        locale == "fr"
    })
    .unwrap();
    assert_eq!(read.len(), 40);
    assert!(read.iter().all(|version| version.locale == "fr"));
    assert_eq!(walks() - before, 1, "one NODES walk for the whole node");
}
