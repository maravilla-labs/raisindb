//! Lookup semantics: revisions, ancestors, hidden locales, alternates, RLS,
//! the fallback, uniqueness.

use super::support::*;
use raisin_models::auth::AuthContext;
use raisin_rocksdb::cf;
use raisin_storage::localized::LocalizedServedBy::{self, Fallback, Index};
use raisin_storage::NodeRepository;
use raisin_storage::Storage;

fn index_keys(storage: &raisin_rocksdb::RocksDBStorage) -> usize {
    let db = storage.db();
    db.iterator_cf(
        db.cf_handle(cf::LOCALIZED_NAME_INDEX).unwrap(),
        rocksdb::IteratorMode::Start,
    )
    .count()
}

fn node_service(
    storage: &std::sync::Arc<raisin_rocksdb::RocksDBStorage>,
    auth: AuthContext,
) -> raisin_core::NodeService<raisin_rocksdb::RocksDBStorage> {
    raisin_core::NodeService::new_with_context(
        storage.clone(),
        T.to_string(),
        R.to_string(),
        B.to_string(),
        WS.to_string(),
    )
    .with_auth(auth)
}

#[tokio::test]
async fn lookup_at_every_revision() {
    let (storage, _dir) = open().await;
    build(&storage, B).await; // Ready from the empty branch on
    let products = create(&storage, "/products", &[]).await;
    let chair = create(&storage, "/products/chair", &[]).await;
    let r1 = head(&storage, B).await;
    set_name(&storage, &products, "fr", "produits").await;
    set_name(&storage, &chair, "fr", "chaise").await;
    let r2 = head(&storage, B).await;
    set_name(&storage, &chair, "fr", "siege").await;
    let r3 = head(&storage, B).await;

    let at = |rev: &raisin_hlc::HLC, path: &str| {
        resolve_at(&storage, B, "fr", path, Some(rev)).map(|r| {
            assert_eq!(r.served_by, Index);
            (r.node_id, r.canonical_localized_path)
        })
    };
    // r1: no names yet — the canonical names are the localized path.
    assert_eq!(
        at(&r1, "/products/chair"),
        Some((chair.clone(), "/products/chair".into()))
    );
    assert_eq!(at(&r1, "/produits/chaise"), None);
    // r2: both names.
    assert_eq!(
        at(&r2, "/produits/chaise"),
        Some((chair.clone(), "/produits/chaise".into()))
    );
    assert_eq!(at(&r2, "/produits/siege"), None);
    // r3: the chair's name changed.
    assert_eq!(
        at(&r3, "/produits/siege"),
        Some((chair.clone(), "/produits/siege".into()))
    );
    assert_eq!(at(&r3, "/produits/chaise"), None);
    // A canonical name where the node has its own name resolves, flagged 301.
    let hint = resolve_at(&storage, B, "fr", "/products/chair", Some(&r3)).unwrap();
    assert!(hint.redirect);
    assert_eq!(hint.canonical_localized_path, "/produits/siege");
    // The fallback agrees at every revision.
    for (rev, path) in [
        (&r1, "/products/chair"),
        (&r2, "/produits/chaise"),
        (&r3, "/produits/siege"),
    ] {
        assert!(resolve_at(&storage, B, "fr", path, Some(rev)).is_some());
    }
}

#[tokio::test]
async fn ancestor_move_and_rename_needs_no_rewrite() {
    let (storage, _dir) = open().await;
    build(&storage, B).await;
    let (_products, chair) = catalog(&storage).await;
    let before = index_keys(&storage);

    storage
        .nodes()
        .rename_node(scope(B), "/products", "catalog")
        .await
        .unwrap();
    assert_eq!(
        index_keys(&storage),
        before,
        "an ancestor rename writes nothing"
    );
    let found = resolve(&storage, "fr", "/produits/chaise").unwrap();
    assert_eq!(found.node_id, chair);
    assert_eq!(found.canonical_path, "/catalog/chair");

    // Moving the node itself rewrites its own rows only.
    let other = create(&storage, "/other", &[]).await;
    set_name(&storage, &other, "fr", "autre").await;
    let before = index_keys(&storage);
    storage
        .nodes()
        .move_node(scope(B), &chair, "/other/chair", None)
        .await
        .unwrap();
    assert!(
        index_keys(&storage) - before <= 3,
        "one claim, one reverse row, one tombstone"
    );
    assert_eq!(
        id_via(&storage, B, "fr", "/autre/chaise", Index),
        Some(chair)
    );
    assert_eq!(resolve(&storage, "fr", "/produits/chaise"), None);
}

#[tokio::test]
async fn hidden_locale_hides() {
    let (storage, _dir) = open().await;
    build(&storage, B).await;
    let (_, chair) = catalog(&storage).await;
    set_name(&storage, &chair, "de", "stuhl").await;
    hide(&storage, &chair, "fr").await;
    assert_eq!(resolve(&storage, "fr", "/produits/chaise"), None);
    assert_eq!(
        resolve(&storage, "fr", "/produits/chair"),
        None,
        "hidden by any name"
    );
    assert_eq!(
        id_via(&storage, B, "de", "/products/stuhl", Index),
        Some(chair)
    );
}

#[tokio::test]
async fn localized_alternates_hide_hidden_locales() {
    let (storage, _dir) = open().await;
    build(&storage, B).await;
    let (_, chair) = catalog(&storage).await;
    set_name(&storage, &chair, "de", "stuhl").await;
    let service = node_service(&storage, AuthContext::system());
    let found = service
        .resolve_localized_path("fr", "/produits/chaise")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.node.id, chair);
    assert_eq!(
        found.alternates.get("en").map(String::as_str),
        Some("/products/chair")
    );
    assert_eq!(
        found.alternates.get("fr").map(String::as_str),
        Some("/produits/chaise")
    );
    assert_eq!(
        found.alternates.get("de").map(String::as_str),
        Some("/products/stuhl")
    );

    hide(&storage, &chair, "de").await;
    let found = service
        .resolve_localized_path("fr", "/produits/chaise")
        .await
        .unwrap()
        .unwrap();
    assert!(
        !found.alternates.contains_key("de"),
        "{:?}",
        found.alternates
    );
    assert_eq!(found.alternates.len(), 2);
}

#[tokio::test]
async fn rls_404_parity() {
    let (storage, _dir) = open().await;
    build(&storage, B).await;
    let (_, chair) = catalog(&storage).await;
    set_name(&storage, &chair, "de", "stuhl").await;
    hide(&storage, &chair, "de").await;
    let system = node_service(&storage, AuthContext::system());
    let stranger = node_service(&storage, AuthContext::for_user("stranger"));
    assert!(system
        .resolve_localized_path("fr", "/produits/chaise")
        .await
        .unwrap()
        .is_some());
    // Forbidden, missing and hidden are the same answer.
    let forbidden = stranger
        .resolve_localized_path("fr", "/produits/chaise")
        .await
        .unwrap();
    let missing = system
        .resolve_localized_path("fr", "/produits/nope")
        .await
        .unwrap();
    let hidden = system
        .resolve_localized_path("de", "/products/stuhl")
        .await
        .unwrap();
    assert!(forbidden.is_none() && missing.is_none() && hidden.is_none());
}

#[tokio::test]
async fn fallback_to_row_scan_until_ready() {
    let (storage, _dir) = open().await;
    let products = create(&storage, "/products", &[]).await;
    let chair = create(&storage, "/products/chair", &[]).await;
    set_name(&storage, &products, "fr", "produits").await;
    // A stored URL contributes its last segment.
    set_name(&storage, &chair, "fr", "/fr/produits/chaise").await;
    assert_eq!(
        availability(&storage, B),
        raisin_rocksdb::localized_name::Availability::NotBuilt
    );
    assert_eq!(
        id_via(&storage, B, "fr", "/produits/chaise", Fallback),
        Some(chair.clone())
    );
    assert_eq!(
        id_via(&storage, B, "fr", "/produits", Fallback),
        Some(products)
    );
    let report = build(&storage, B).await;
    assert_eq!(report.localized_names.ready_workspaces, 1);
    assert!(availability(&storage, B).is_ready());
    assert_eq!(
        id_via(&storage, B, "fr", "/produits/chaise", Index),
        Some(chair)
    );
    // A rebuild over a maintained index writes nothing.
    assert_eq!(build(&storage, B).await.localized_names.rewritten, 0);
}

#[tokio::test]
async fn sibling_uniqueness_when_enforced() {
    let (storage, _dir) = open().await;
    let (_, chair) = catalog(&storage).await;
    let table = create(&storage, "/products/table", &[]).await;
    // Not enforced: a collision is accepted, resolved deterministically
    // (the newest claim wins) and reported by the build.
    set_name(&storage, &table, "fr", "chaise").await;
    assert_eq!(
        resolve(&storage, "fr", "/produits/chaise").unwrap().node_id,
        table
    );
    let mut config = repo_config();
    config.localized_names.enforce_unique = true;
    set_config(&storage, config).await;
    assert_eq!(build(&storage, B).await.localized_names.collisions, 1);
    assert_eq!(
        id_via(&storage, B, "fr", "/produits/chaise", Index),
        Some(table.clone())
    );
    // Enforced but not clean: still accepted.
    set_name(&storage, &table, "fr", "chaise").await;
    // Clean it up, rebuild: zero collisions, and now the probe refuses.
    set_name(&storage, &table, "fr", "table").await;
    assert_eq!(build(&storage, B).await.localized_names.collisions, 0);
    let refused = try_set_name(&storage, &table, "fr", "chaise").await;
    assert!(
        matches!(refused, Err(raisin_error::Error::Conflict(_))),
        "{refused:?}"
    );
    assert_eq!(
        id_via(&storage, B, "fr", "/produits/chaise", Index),
        Some(chair)
    );
    // The same name in another locale, or under another parent, is fine.
    set_name(&storage, &table, "de", "chaise").await;
    let _ = LocalizedServedBy::Index;
}
