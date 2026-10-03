//! A tenant whose id is also a kind-first record kind.
//!
//! A mixed-layout column family (`cf::INDEX_STATUS`) holds tenant-first
//! records, `{tenant}\0{repo}\0…`, beside kind-first ones,
//! `{kind}\0{tenant}\0{repo}\0…`. The whole-tenant wipe removes the
//! tenant-first ones with one `{tenant}\0` range — and for a tenant NAMED
//! `prop_index`, `compound_index` or `spatial_index`, that range is exactly
//! every OTHER tenant's kind-first records of that kind. Nothing at tenant
//! creation forbids those ids, so the wipe cannot rely on it.
//!
//! For such a tenant the tenant-first records are removed per repository
//! instead (`{tenant}\0{repo}\0`, repositories read from the registry BEFORE
//! the registry itself is wiped). One ambiguity is inherent in the key format
//! and remains: a repository of this tenant named like another tenant shares a
//! prefix with that tenant's kind-first records. Losing a state record there
//! reads as "not built" — the index rebuilds and nothing serves stale rows.

use rocksdb::DB;

use raisin_error::Result;

use crate::storage::repo_purge::INDEX_STATUS_KINDS;
use crate::{cf, cf_handle, keys};

/// Whether `tenant`'s `{tenant}\0` range would reach kind-first records.
pub(super) fn collides_with_kind(tenant: &str) -> bool {
    INDEX_STATUS_KINDS.contains(&tenant)
}

/// `tenant`'s repositories, from the registry's `{tenant}\0repos\0{repo}`.
pub(super) fn tenant_repos(db: &DB, tenant: &str) -> Result<Vec<String>> {
    let cf = cf_handle(db, cf::REGISTRY)?;
    let prefix = keys::KeyBuilder::new()
        .push(tenant)
        .push("repos")
        .build_prefix();
    let mut repos = Vec::new();
    for item in crate::prefix_scan(db, cf, &prefix) {
        let (key, _) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;
        if !key.starts_with(&prefix) {
            break;
        }
        let rest = &key[prefix.len()..];
        let repo = rest.split(|&b| b == 0).next().unwrap_or(rest);
        repos.push(String::from_utf8_lossy(repo).into_owned());
    }
    Ok(repos)
}

/// Remove `tenant`'s tenant-first records from `cf_name` one repository at a
/// time, never touching a `{kind}\0…` record outside those repositories.
pub(super) fn wipe_tenant_first_per_repo(
    db: &DB,
    cf_name: &str,
    tenant: &str,
    repos: &[String],
) -> Result<()> {
    let cf = cf_handle(db, cf_name)?;
    for repo in repos {
        let lo = keys::KeyBuilder::new()
            .push(tenant)
            .push(repo)
            .build_prefix();
        let Some(hi) = crate::prefix_successor(&lo) else {
            continue;
        };
        db.delete_range_cf(cf, &lo, &hi).map_err(|e| {
            raisin_error::Error::storage(format!("delete_range_cf({cf_name}) failed: {e}"))
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::{cf, cf_handle, RocksDBStorage};

    /// Wiping a tenant NAMED `compound_index` removes its own records and
    /// leaves every other tenant's `compound_index\0…` state alone.
    #[test]
    fn tenant_named_like_a_kind_does_not_wipe_other_tenants() {
        let dir = tempfile::tempdir().unwrap();
        let storage = RocksDBStorage::new(dir.path()).unwrap();
        let db = storage.db().clone();
        let put = |cf_name: &str, key: &str| {
            db.put_cf(cf_handle(&db, cf_name).unwrap(), key.as_bytes(), b"{}")
                .unwrap()
        };
        let has = |key: &str| {
            db.get_cf(cf_handle(&db, cf::INDEX_STATUS).unwrap(), key.as_bytes())
                .unwrap()
                .is_some()
        };

        let tenant = "compound_index";
        put(cf::REGISTRY, "compound_index\0repos\0site");
        let own = "compound_index\0site\0main\0repair_state\0ordered_children\0n1";
        let others = "compound_index\0acme\0shop\0main\0ws\0by_cat";
        put(cf::INDEX_STATUS, own);
        put(cf::INDEX_STATUS, others);

        storage.delete_tenant_data(tenant).unwrap();

        assert!(!has(own), "the wiped tenant's own record survived");
        assert!(has(others), "another tenant's compound state was wiped");
    }
}
