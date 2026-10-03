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
        DB::open_cf(&opts, dir.path(), [cf::TRANSLATION_DATA]).unwrap(),
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
