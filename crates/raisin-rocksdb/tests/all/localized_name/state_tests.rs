//! The fingerprinted, fail-closed build state: default-language changes and
//! forks fall back until rebuilt.

use super::support::*;
use raisin_rocksdb::localized_name::Availability;
use raisin_storage::localized::LocalizedServedBy::{DefaultLanguage, Fallback, Index};
use raisin_storage::BranchRepository;
use raisin_storage::Storage;

/// `/products/chair` with translated names in `en` and `fr`, built.
async fn bilingual() -> (
    std::sync::Arc<raisin_rocksdb::RocksDBStorage>,
    tempfile::TempDir,
    String,
) {
    let (storage, dir) = open().await;
    let products = create(&storage, "/products", &[]).await;
    let chair = create(&storage, "/products/chair", &[]).await;
    for (id, en, fr) in [(&products, "goods", "produits"), (&chair, "seat", "chaise")] {
        set_name(&storage, id, "en", en).await;
        set_name(&storage, id, "fr", fr).await;
    }
    build(&storage, B).await;
    assert!(availability(&storage, B).is_ready());
    (storage, dir, chair)
}

async fn make_default(storage: &raisin_rocksdb::RocksDBStorage, language: &str) {
    let mut config = repo_config();
    config.set_default_language(language);
    set_config(storage, config).await;
}

#[tokio::test]
async fn default_language_change_falls_back_until_rebuilt() {
    let (storage, _dir, chair) = bilingual().await;
    assert_eq!(
        id_via(&storage, B, "fr", "/produits/chaise", Index),
        Some(chair.clone())
    );

    make_default(&storage, "fr").await;
    // Flipped in the same batch as the config: not ready, and lookups are
    // still CORRECT under the new configuration — through the fallback.
    assert_eq!(availability(&storage, B), Availability::NotBuilt);
    assert_eq!(
        id_via(&storage, B, "en", "/goods/seat", Fallback),
        Some(chair.clone())
    );
    assert_eq!(
        id_via(&storage, B, "fr", "/products/chair", DefaultLanguage),
        Some(chair.clone())
    );
    assert_eq!(
        resolve(&storage, "fr", "/produits/chaise"),
        None,
        "fr is canonical now"
    );

    build(&storage, B).await;
    assert!(availability(&storage, B).is_ready());
    assert_eq!(id_via(&storage, B, "en", "/goods/seat", Index), Some(chair));
}

#[tokio::test]
async fn default_language_change_rekeys() {
    let (storage, _dir, chair) = bilingual().await;
    let products = resolve(&storage, "fr", "/produits").unwrap().node_id;
    // Before: en is the default, so only fr is keyed.
    assert!(claimants(&storage, B, "en", "/", "goods").is_empty());
    assert_eq!(
        claimants(&storage, B, "fr", "/", "produits"),
        vec![products.clone()]
    );

    make_default(&storage, "fr").await;
    // The inline writers re-key a node on its next write; the rebuild
    // re-keys every node.
    build(&storage, B).await;
    assert_eq!(
        claimants(&storage, B, "en", "/", "goods"),
        vec![products.clone()]
    );
    assert_eq!(claimants(&storage, B, "en", &products, "seat"), vec![chair]);
    assert!(
        claimants(&storage, B, "fr", "/", "produits").is_empty(),
        "the new default language has no localized names"
    );
}

#[tokio::test]
async fn fork_is_not_ready_until_rebuilt() {
    let (storage, _dir, chair) = bilingual().await;
    storage
        .branches()
        .create_branch(
            T,
            R,
            "feature",
            "test",
            None,
            Some(B.to_string()),
            false,
            false,
        )
        .await
        .unwrap();
    // INDEX_STATUS is not forked: the fork has no state record of its own.
    assert_eq!(availability(&storage, "feature"), Availability::NotBuilt);
    assert_eq!(
        id_via(&storage, "feature", "fr", "/produits/chaise", Fallback),
        Some(chair.clone())
    );
    // The claims WERE forked with the data: the fork's build writes nothing.
    let report = build(&storage, "feature").await;
    assert_eq!(report.localized_names.rewritten, 0, "{report:?}");
    assert!(availability(&storage, "feature").is_ready());
    assert_eq!(
        id_via(&storage, "feature", "fr", "/produits/chaise", Index),
        Some(chair)
    );
    // The source stays ready.
    assert!(availability(&storage, B).is_ready());
}
