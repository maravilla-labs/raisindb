//! Phase 12.0: the column family's registration, tenant wipe coverage and
//! the downgrade boundary.

use super::support::*;
use raisin_rocksdb::{cf, RocksDBStorage};
use rocksdb::{ColumnFamilyDescriptor, Options, DB};
use std::sync::Arc;

fn cf_keys(db: &DB, tenant: &str) -> usize {
    db.iterator_cf(
        db.cf_handle(cf::LOCALIZED_NAME_INDEX).unwrap(),
        rocksdb::IteratorMode::Start,
    )
    .filter(|item| {
        item.as_ref()
            .is_ok_and(|(k, _)| k.starts_with(format!("{tenant}\0").as_bytes()))
    })
    .count()
}

#[tokio::test]
async fn tenant_wipe_lists_cf() {
    let (storage, _dir) = open().await;
    catalog(&storage).await;
    assert!(cf_keys(storage.db(), T) > 0);
    storage.delete_tenant_data(T).unwrap();
    assert_eq!(
        cf_keys(storage.db(), T),
        0,
        "TENANT_PREFIXED_CFS must list the CF"
    );
}

#[tokio::test]
async fn downgrade_to_registration_release_opens_db() {
    let dir = tempfile::TempDir::new().unwrap();
    let chair = {
        let storage = Arc::new(RocksDBStorage::new(dir.path()).unwrap());
        provision(&storage).await;
        let (_, chair) = catalog(&storage).await;
        chair
    };
    // The registration release's open: its CF list (which declares the CF),
    // no writer. The data written by the writer release stays readable.
    let db = raisin_rocksdb::open_db(dir.path()).unwrap();
    assert!(db.cf_handle(cf::LOCALIZED_NAME_INDEX).is_some());
    drop(db);
    let storage = RocksDBStorage::new(dir.path()).unwrap();
    assert_eq!(
        resolve(&storage, "fr", "/produits/chaise").unwrap().node_id,
        chair
    );
}

#[tokio::test]
async fn downgrade_open_previous_release() {
    let dir = tempfile::TempDir::new().unwrap();
    {
        let storage = RocksDBStorage::new(dir.path()).unwrap();
        provision(&storage).await;
    }
    // (a) Below the registration release a DB holding the CF does not open:
    // RocksDB requires every on-disk CF to be named, and the previous
    // release's list lacks this one. That is the documented boundary
    // ("downgrade unsupported below 12.0").
    let previous: Vec<&str> = raisin_rocksdb::all_column_family_names()
        .into_iter()
        .filter(|name| *name != cf::LOCALIZED_NAME_INDEX)
        .collect();
    let mut opts = Options::default();
    opts.create_if_missing(false);
    let refused = DB::open_cf_descriptors(
        &opts,
        dir.path(),
        previous
            .iter()
            .map(|name| ColumnFamilyDescriptor::new(*name, Options::default())),
    );
    assert!(
        refused.is_err(),
        "the previous release's CF list must not open it"
    );

    // (b) From this release on, a CF a NEWER release added is opened
    // generically and kept: a later downgrade to this release still works.
    {
        let mut opts = Options::default();
        opts.create_missing_column_families(true);
        let mut names: Vec<String> = DB::list_cf(&opts, dir.path()).unwrap();
        names.push("from_a_newer_release".to_string());
        let db = DB::open_cf(&opts, dir.path(), &names).unwrap();
        let extra = db.cf_handle("from_a_newer_release").unwrap();
        db.put_cf(extra, b"k", b"v").unwrap();
    }
    let storage = RocksDBStorage::new(dir.path()).unwrap();
    let extra = storage.db().cf_handle("from_a_newer_release").unwrap();
    assert_eq!(
        storage.db().get_cf(extra, b"k").unwrap().as_deref(),
        Some(&b"v"[..])
    );
}
