//! Plan Phase 11 items 1, 2 and the history floor: revision-correct overlay
//! reads, atomic writes, and locale-scoped time travel below the floor.
//!
//! The repository read HEAD whatever revision it was asked for, so time
//! travel with a locale showed today's translation; the batch reader failed
//! on a tombstone; a store was five separate puts.

use raisin_hlc::HLC;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::translations::{JsonPointer, LocaleCode, LocaleOverlay, TranslationMeta};
use raisin_rocksdb::{cf, translation_history, RocksDBStorage};
use raisin_storage::{Storage, TranslationRepository};
use std::collections::HashMap;
use tempfile::TempDir;

pub(crate) const T: &str = "tr-tenant";
pub(crate) const R: &str = "tr-repo";
pub(crate) const B: &str = "main";
pub(crate) const WS: &str = "content";
const NODE: &str = "page-1";

/// Realistic revisions: their descending encodings are not valid UTF-8.
pub(crate) fn rev(n: u64) -> HLC {
    HLC::new(1_705_843_009_000 + n, 0)
}

pub(crate) fn title(text: &str) -> LocaleOverlay {
    let mut data = HashMap::new();
    data.insert(
        JsonPointer::new("/title"),
        PropertyValue::String(text.to_string()),
    );
    LocaleOverlay::Properties { data }
}

pub(crate) fn code(locale: &str) -> LocaleCode {
    LocaleCode::parse(locale).unwrap()
}

pub(crate) fn meta(locale: &str, revision: HLC) -> TranslationMeta {
    TranslationMeta {
        locale: code(locale),
        revision,
        parent_revision: None,
        timestamp: chrono::Utc::now(),
        actor: "translator".to_string(),
        message: "translate".to_string(),
        is_system: false,
    }
}

pub(crate) async fn store(
    storage: &RocksDBStorage,
    node: &str,
    locale: &str,
    overlay: LocaleOverlay,
    revision: HLC,
) {
    storage
        .translations()
        .store_translation(
            T,
            R,
            B,
            WS,
            node,
            &code(locale),
            &overlay,
            &meta(locale, revision),
        )
        .await
        .unwrap();
}

/// A translation tombstone for one locale: a `T` version and a `T` index
/// entry (what `translation_write::stage_version` stores for `None`), raw.
pub(crate) fn tombstone(storage: &RocksDBStorage, node: &str, locale: &str, revision: &HLC) {
    let db = storage.db();
    let mut key = format!("{T}\0{R}\0{B}\0{WS}\0translations\0{node}\0{locale}\0").into_bytes();
    key.extend_from_slice(&revision.encode_descending());
    db.put_cf(db.cf_handle(cf::TRANSLATION_DATA).unwrap(), key, b"T")
        .unwrap();
    let mut index = format!("{T}\0{R}\0translation_index\0{locale}\0").into_bytes();
    index.extend_from_slice(&revision.encode_descending());
    index.push(0);
    index.extend_from_slice(node.as_bytes());
    db.put_cf(db.cf_handle(cf::TRANSLATION_INDEX).unwrap(), index, b"T")
        .unwrap();
}

pub(crate) async fn get(
    storage: &RocksDBStorage,
    node: &str,
    locale: &str,
    at: HLC,
) -> raisin_error::Result<Option<LocaleOverlay>> {
    storage
        .translations()
        .get_translation(T, R, B, WS, node, &code(locale), &at)
        .await
}

fn open() -> (RocksDBStorage, TempDir) {
    let dir = TempDir::new().unwrap();
    (RocksDBStorage::new(dir.path()).unwrap(), dir)
}

async fn locales_at(storage: &RocksDBStorage, at: HLC) -> Vec<String> {
    let mut locales: Vec<String> = storage
        .translations()
        .list_translations_for_node(T, R, B, WS, NODE, &at)
        .await
        .unwrap()
        .iter()
        .map(|l| l.as_str().to_string())
        .collect();
    locales.sort();
    locales
}

async fn translated_nodes_at(storage: &RocksDBStorage, at: HLC) -> Vec<String> {
    storage
        .translations()
        .list_nodes_with_translation(T, R, B, WS, &code("fr"), &at)
        .await
        .unwrap()
}

#[tokio::test]
async fn overlay_read_at_revision() {
    let (storage, _dir) = open();
    store(&storage, NODE, "fr", title("v1"), rev(1)).await;
    store(&storage, NODE, "fr", title("v2"), rev(3)).await;
    tombstone(&storage, NODE, "fr", &rev(5));
    store(&storage, NODE, "fr", LocaleOverlay::Hidden, rev(7)).await;

    let expected = [
        (0, None),
        (1, Some(title("v1"))),
        (2, Some(title("v1"))),
        (3, Some(title("v2"))),
        (4, Some(title("v2"))),
        (5, None),
        (6, None),
        (7, Some(LocaleOverlay::Hidden)),
    ];
    for (n, want) in expected {
        assert_eq!(
            get(&storage, NODE, "fr", rev(n)).await.unwrap(),
            want,
            "at r{n}"
        );
    }

    assert!(locales_at(&storage, rev(0)).await.is_empty());
    assert_eq!(locales_at(&storage, rev(2)).await, vec!["fr"]);
    assert!(locales_at(&storage, rev(5)).await.is_empty());
    assert_eq!(locales_at(&storage, rev(7)).await, vec!["fr"]);

    assert!(translated_nodes_at(&storage, rev(0)).await.is_empty());
    assert_eq!(translated_nodes_at(&storage, rev(2)).await, vec![NODE]);
    assert!(translated_nodes_at(&storage, rev(5)).await.is_empty());
    assert_eq!(translated_nodes_at(&storage, rev(7)).await, vec![NODE]);

    // Block overlays answer to the same rule.
    let repo = storage.translations();
    for (n, text) in [(1, "b1"), (3, "b2")] {
        repo.store_block_translation(
            T,
            R,
            B,
            WS,
            NODE,
            "block-1",
            &code("fr"),
            &title(text),
            &meta("fr", rev(n)),
        )
        .await
        .unwrap();
    }
    let block_at = |n: u64| {
        let repo = repo.clone();
        async move {
            repo.get_block_translation(T, R, B, WS, NODE, "block-1", &code("fr"), &rev(n))
                .await
                .unwrap()
        }
    };
    assert_eq!(block_at(0).await, None);
    assert_eq!(block_at(2).await, Some(title("b1")));
    assert_eq!(block_at(3).await, Some(title("b2")));
}

#[tokio::test]
async fn batch_overlay_read_with_tombstone() {
    let (storage, _dir) = open();
    store(&storage, "n1", "fr", title("one"), rev(1)).await;
    store(&storage, "n2", "fr", title("two"), rev(1)).await;
    tombstone(&storage, "n2", "fr", &rev(2));
    store(&storage, "n3", "fr", LocaleOverlay::Hidden, rev(1)).await;

    let ids: Vec<String> = ["n1", "n2", "n3", "n4"].map(String::from).to_vec();
    let batch = |n: u64| {
        let ids = ids.clone();
        let repo = storage.translations().clone();
        async move {
            repo.get_translations_batch(T, R, B, WS, &ids, &code("fr"), &rev(n))
                .await
                .expect("a tombstone must not fail the batch")
        }
    };

    let at_r2 = batch(2).await;
    assert_eq!(at_r2.len(), 2);
    assert_eq!(at_r2.get("n1"), Some(&title("one")));
    assert_eq!(at_r2.get("n3"), Some(&LocaleOverlay::Hidden));
    assert!(!at_r2.contains_key("n2"), "deleted at r2");

    let at_r1 = batch(1).await;
    assert_eq!(
        at_r1.get("n2"),
        Some(&title("two")),
        "live before its delete"
    );
    assert!(batch(0).await.is_empty());
}

/// Every record of a repository translation write lands in ONE WriteBatch:
/// read the WAL after the store and count the batches.
#[tokio::test]
async fn translation_write_is_atomic() {
    let (storage, _dir) = open();
    let db = storage.db().clone();

    let wal_batches = |since: u64| -> Vec<usize> {
        db.get_updates_since(since)
            .unwrap()
            .map(|item| item.unwrap().1.len())
            .collect()
    };

    let since = db.latest_sequence_number();
    store(&storage, NODE, "fr", title("v1"), rev(1)).await;
    // Version, index entry, translation meta, snapshot, revision meta.
    assert_eq!(wal_batches(since), vec![5]);

    let since = db.latest_sequence_number();
    storage
        .translations()
        .store_block_translation(
            T,
            R,
            B,
            WS,
            NODE,
            "block-1",
            &code("fr"),
            &title("b"),
            &meta("fr", rev(2)),
        )
        .await
        .unwrap();
    // Block version (no index entry, no meta), snapshot, revision meta.
    assert_eq!(wal_batches(since), vec![3]);
}

#[tokio::test]
async fn locale_time_travel_below_history_floor_fails_loudly() {
    let (storage, _dir) = open();
    store(&storage, NODE, "fr", title("v1"), rev(1)).await;
    store(&storage, NODE, "fr", title("v5"), rev(5)).await;
    let db = storage.db();

    // No floor: full history.
    assert_eq!(
        get(&storage, NODE, "fr", rev(2)).await.unwrap(),
        Some(title("v1"))
    );

    translation_history::raise_complete_from(db, T, R, B, rev(3)).unwrap();
    // A lower floor never lowers it.
    translation_history::raise_complete_from(db, T, R, B, rev(2)).unwrap();
    assert_eq!(
        translation_history::complete_from(db, T, R, B).unwrap(),
        Some(rev(3))
    );

    let err = get(&storage, NODE, "fr", rev(2)).await.unwrap_err();
    assert!(
        err.to_string().contains("complete only from"),
        "must say why: {err}"
    );
    let repo = storage.translations();
    assert!(repo
        .get_translations_batch(T, R, B, WS, &[NODE.to_string()], &code("fr"), &rev(2))
        .await
        .is_err());
    assert!(repo
        .list_translations_for_node(T, R, B, WS, NODE, &rev(2))
        .await
        .is_err());
    assert!(repo
        .list_nodes_with_translation(T, R, B, WS, &code("fr"), &rev(2))
        .await
        .is_err());

    // At and above the floor, reads answer.
    assert_eq!(
        get(&storage, NODE, "fr", rev(3)).await.unwrap(),
        Some(title("v1"))
    );
    assert_eq!(
        get(&storage, NODE, "fr", rev(9)).await.unwrap(),
        Some(title("v5"))
    );
    // Another repository, and another branch, are not affected.
    assert!(repo
        .get_translation(T, "other", B, WS, NODE, &code("fr"), &rev(1))
        .await
        .is_ok());
    assert!(repo
        .get_translation(T, R, "feature", WS, NODE, &code("fr"), &rev(1))
        .await
        .is_ok());
}
