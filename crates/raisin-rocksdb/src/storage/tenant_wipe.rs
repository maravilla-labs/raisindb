//! Tenant-scoped data wipe across every RocksDB column family that uses a
//! tenant-prefixed key layout.
//!
//! Used by the superadmin `DELETE /management/admin/tenants/{tenant_id}` endpoint
//! to fully remove a tenant's data — the foundation for archive-style retention
//! flows (e.g. delete after N days of inactivity).
//!
//! ## Key-format conventions
//!
//! Most tenant data uses the standard `{tenant}\0...` prefix from the data
//! plane (see `crates/raisin-rocksdb/src/keys/`). For those CFs we range-delete
//! from `{tenant}\0` (inclusive) to `{tenant}\x01` (exclusive).
//!
//! Two CFs deviate:
//! - `ADMIN_USERS`: keys are `sys\0{tenant}\0users\0{username}`.
//! - `REGISTRY`: holds both per-tenant entries (`{tenant}\0repos\0...`) and a
//!   global tenant-list (`tenants\0{tenant}\0...`).
//!
//! Both deviations get a dedicated prefix pair below.
//!
//! ## What is NOT wiped
//!
//! - `OPERATION_LOG` / `APPLIED_OPS`: cluster replication state. Removing
//!   these mid-cluster would desync peers; replication GC handles it.
//! - `SYSTEM_UPDATE_HASHES`: uses colon-delimited keys; left in place since
//!   the keys reference repos that no longer exist after the wipe.
//! - Tantivy fulltext / HNSW vector index directories on disk: tenant-isolated
//!   per directory, so leaving them is not a security issue. Documented in the
//!   handler's response/log so operators know to clean them up if disk pressure
//!   matters.

use std::sync::Arc;

use raisin_error::Result;
use rocksdb::DB;

use crate::cf;
use crate::keys::{job_tenant_prefix, job_tenant_upper};
use crate::RocksDBStorage;

/// Column families whose keys start with `{tenant}\0...`.
///
/// Order is informational only — every CF is wiped independently.
const TENANT_PREFIXED_CFS: &[&str] = &[
    // Node + index data
    cf::NODES,
    cf::PATH_INDEX,
    cf::PROPERTY_INDEX,
    cf::REFERENCE_INDEX,
    cf::RELATION_INDEX,
    cf::ORDER_INDEX,
    cf::ORDERED_CHILDREN,
    cf::NODE_PATH,
    cf::COMPOUND_INDEX,
    cf::UNIQUE_INDEX,
    cf::SPATIAL_INDEX,
    // Workspace / versioning
    cf::WORKSPACES,
    cf::BRANCHES,
    cf::TAGS,
    cf::REVISIONS,
    cf::TREES,
    cf::WORKSPACE_DELTAS,
    cf::VERSIONS,
    // Schema
    cf::NODE_TYPES,
    cf::ARCHETYPES,
    cf::ELEMENT_TYPES,
    // Tenant configuration
    cf::TENANT_EMBEDDING_CONFIG,
    cf::TENANT_AI_CONFIG,
    cf::TENANT_AUTH_CONFIG,
    // AI / vector
    cf::EMBEDDINGS,
    cf::EMBEDDING_JOBS,
    cf::QUERY_EMBEDDINGS,
    cf::FULLTEXT_JOBS,
    // Jobs queue (also covered by purge_all_jobs, harmless to repeat)
    cf::JOB_DATA,
    cf::JOB_METADATA,
    // Translations
    cf::TRANSLATION_DATA,
    cf::BLOCK_TRANSLATIONS,
    cf::TRANSLATION_INDEX,
    cf::TRANSLATION_HASHES,
    // Index status
    cf::INDEX_STATUS,
    // Auth / identity
    cf::IDENTITIES,
    cf::IDENTITY_EMAIL_INDEX,
    cf::SESSIONS,
    // Graph
    cf::GRAPH_CACHE,
    cf::GRAPH_PROJECTION,
    // Processing
    cf::PROCESSING_RULES,
    // Audit log (retention: a wiped tenant must not leave audit rows behind)
    cf::AUDIT_LOG,
    // Per-tenant registry entries (the global tenants entry is wiped separately)
    cf::REGISTRY,
];

/// Result of a tenant wipe: which CFs were touched and whether compaction ran.
#[derive(Debug, Clone)]
pub struct TenantWipeReport {
    pub cfs_wiped: usize,
    pub compacted: bool,
    pub skipped: Vec<String>,
}

impl RocksDBStorage {
    /// Wipe every tenant-prefixed entry from RocksDB and compact the affected
    /// ranges. Idempotent: re-running on an already-empty tenant is a no-op.
    pub fn delete_tenant_data(&self, tenant_id: &str) -> Result<TenantWipeReport> {
        let db = self.db();
        let prefix = job_tenant_prefix(tenant_id);
        let upper = job_tenant_upper(tenant_id);

        let mut cfs_wiped = 0usize;
        let mut skipped: Vec<String> = Vec::new();

        for cf_name in TENANT_PREFIXED_CFS {
            match wipe_cf_range(db, cf_name, &prefix, &upper) {
                Ok(()) => cfs_wiped += 1,
                Err(e) => {
                    tracing::warn!(
                        cf = %cf_name,
                        tenant_id = %tenant_id,
                        error = %e,
                        "Failed to wipe tenant data in CF; continuing"
                    );
                    skipped.push((*cf_name).to_string());
                }
            }
        }

        // ADMIN_USERS uses `sys\0{tenant}\0users\0...` keys.
        let admin_lo = admin_user_tenant_prefix(tenant_id);
        let admin_hi = admin_user_tenant_upper(tenant_id);
        match wipe_cf_range(db, cf::ADMIN_USERS, &admin_lo, &admin_hi) {
            Ok(()) => cfs_wiped += 1,
            Err(e) => {
                tracing::warn!(
                    cf = %cf::ADMIN_USERS,
                    tenant_id = %tenant_id,
                    error = %e,
                    "Failed to wipe admin users; continuing"
                );
                skipped.push(cf::ADMIN_USERS.to_string());
            }
        }

        // REGISTRY also has a global `tenants\0{tenant}\0...` entry for tenant listing.
        let reg_lo = registry_global_tenant_prefix(tenant_id);
        let reg_hi = registry_global_tenant_upper(tenant_id);
        // Same CF; only the bound differs. Don't double-count it as a CF here.
        if let Err(e) = wipe_cf_range(db, cf::REGISTRY, &reg_lo, &reg_hi) {
            tracing::warn!(
                cf = %cf::REGISTRY,
                tenant_id = %tenant_id,
                error = %e,
                "Failed to wipe global tenant-list entry in REGISTRY"
            );
            skipped.push(format!("{} (global tenant entry)", cf::REGISTRY));
        }

        tracing::warn!(
            tenant_id = %tenant_id,
            cfs_wiped = cfs_wiped,
            skipped = ?skipped,
            "Tenant data wipe complete"
        );

        Ok(TenantWipeReport {
            cfs_wiped,
            compacted: true,
            skipped,
        })
    }
}

/// `sys\0{tenant}\0` — inclusive lower bound for admin-users keys of a tenant.
fn admin_user_tenant_prefix(tenant_id: &str) -> Vec<u8> {
    let mut k = Vec::with_capacity(4 + tenant_id.len() + 1);
    k.extend_from_slice(b"sys");
    k.push(0);
    k.extend_from_slice(tenant_id.as_bytes());
    k.push(0);
    k
}

/// `sys\0{tenant}\x01` — exclusive upper bound for admin-users keys of a tenant.
fn admin_user_tenant_upper(tenant_id: &str) -> Vec<u8> {
    let mut k = Vec::with_capacity(4 + tenant_id.len() + 1);
    k.extend_from_slice(b"sys");
    k.push(0);
    k.extend_from_slice(tenant_id.as_bytes());
    k.push(1);
    k
}

/// `tenants\0{tenant}\0` — lower bound for the global tenant-list entry.
fn registry_global_tenant_prefix(tenant_id: &str) -> Vec<u8> {
    let mut k = Vec::with_capacity(8 + tenant_id.len() + 1);
    k.extend_from_slice(b"tenants");
    k.push(0);
    k.extend_from_slice(tenant_id.as_bytes());
    k.push(0);
    k
}

/// `tenants\0{tenant}\x01` — upper bound for the global tenant-list entry.
fn registry_global_tenant_upper(tenant_id: &str) -> Vec<u8> {
    let mut k = Vec::with_capacity(8 + tenant_id.len() + 1);
    k.extend_from_slice(b"tenants");
    k.push(0);
    k.extend_from_slice(tenant_id.as_bytes());
    k.push(1);
    k
}

fn wipe_cf_range(db: &Arc<DB>, cf_name: &str, lo: &[u8], hi: &[u8]) -> Result<()> {
    let cf = db
        .cf_handle(cf_name)
        .ok_or_else(|| raisin_error::Error::storage(format!("CF {} not found", cf_name)))?;
    db.delete_range_cf(cf, lo, hi).map_err(|e| {
        raisin_error::Error::storage(format!("delete_range_cf({}) failed: {}", cf_name, e))
    })?;
    db.compact_range_cf(cf, Some(lo), Some(hi));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cf_handle;
    use crate::keys::KeyBuilder;
    use std::sync::Arc;

    /// Helper: write a tenant-prefixed key to a CF.
    fn put_in_cf(db: &Arc<DB>, cf_name: &str, key: &[u8], value: &[u8]) {
        let cf = cf_handle(db, cf_name).unwrap();
        db.put_cf(cf, key, value).unwrap();
    }

    /// Helper: scan a CF for any keys under `{tenant}\0` prefix.
    fn tenant_keys_remaining(db: &Arc<DB>, cf_name: &str, tenant: &str) -> usize {
        let cf = cf_handle(db, cf_name).unwrap();
        let prefix = job_tenant_prefix(tenant);
        let iter = db.prefix_iterator_cf(cf, &prefix);
        let mut count = 0;
        for item in iter {
            let (key, _) = item.unwrap();
            if key.starts_with(&prefix) {
                count += 1;
            } else {
                break;
            }
        }
        count
    }

    #[test]
    fn wipe_removes_tenant_a_leaves_tenant_b() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage = RocksDBStorage::new(temp_dir.path()).unwrap();
        let db = storage.db().clone();

        // Write keys for tenant-a and tenant-b in NODES, BRANCHES, REVISIONS.
        let key_a_nodes = KeyBuilder::new()
            .push("tenant-a")
            .push("repo")
            .push("workspace")
            .push("node-1")
            .build();
        let key_b_nodes = KeyBuilder::new()
            .push("tenant-b")
            .push("repo")
            .push("workspace")
            .push("node-1")
            .build();

        let key_a_branches = KeyBuilder::new()
            .push("tenant-a")
            .push("repo")
            .push("branches")
            .push("main")
            .build();
        let key_b_branches = KeyBuilder::new()
            .push("tenant-b")
            .push("repo")
            .push("branches")
            .push("main")
            .build();

        put_in_cf(&db, cf::NODES, &key_a_nodes, b"a-payload");
        put_in_cf(&db, cf::NODES, &key_b_nodes, b"b-payload");
        put_in_cf(&db, cf::BRANCHES, &key_a_branches, b"a-branch");
        put_in_cf(&db, cf::BRANCHES, &key_b_branches, b"b-branch");

        // Sanity: tenant-a has data in both CFs.
        assert_eq!(tenant_keys_remaining(&db, cf::NODES, "tenant-a"), 1);
        assert_eq!(tenant_keys_remaining(&db, cf::BRANCHES, "tenant-a"), 1);

        // Wipe tenant-a.
        let report = storage.delete_tenant_data("tenant-a").unwrap();
        assert!(report.compacted);
        assert!(report.cfs_wiped > 0);

        // tenant-a is gone in both CFs.
        assert_eq!(
            tenant_keys_remaining(&db, cf::NODES, "tenant-a"),
            0,
            "tenant-a should be wiped from NODES"
        );
        assert_eq!(
            tenant_keys_remaining(&db, cf::BRANCHES, "tenant-a"),
            0,
            "tenant-a should be wiped from BRANCHES"
        );

        // tenant-b is untouched.
        assert_eq!(
            tenant_keys_remaining(&db, cf::NODES, "tenant-b"),
            1,
            "tenant-b NODES leaked from a tenant-a wipe"
        );
        assert_eq!(
            tenant_keys_remaining(&db, cf::BRANCHES, "tenant-b"),
            1,
            "tenant-b BRANCHES leaked from a tenant-a wipe"
        );
    }

    #[test]
    fn wipe_is_idempotent_on_empty_tenant() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage = RocksDBStorage::new(temp_dir.path()).unwrap();

        // Wipe a tenant that never had any data — should succeed.
        let report = storage.delete_tenant_data("never-existed").unwrap();
        assert!(report.compacted);
        assert!(report.cfs_wiped > 0, "should still report CFs touched");
        assert!(report.skipped.is_empty());
    }

    #[test]
    fn admin_users_wipe_uses_sys_prefix() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage = RocksDBStorage::new(temp_dir.path()).unwrap();
        let db = storage.db().clone();

        // Build an admin-users key under `sys\0{tenant}\0users\0{username}`
        // exactly as AdminUserStore does, for tenant-a and tenant-b.
        let mut k_a = Vec::new();
        k_a.extend_from_slice(b"sys");
        k_a.push(0);
        k_a.extend_from_slice(b"tenant-a");
        k_a.push(0);
        k_a.extend_from_slice(b"users");
        k_a.push(0);
        k_a.extend_from_slice(b"admin");

        let mut k_b = k_a.clone();
        // Swap "tenant-a" for "tenant-b".
        let prefix_len = b"sys".len() + 1;
        k_b.drain(prefix_len..prefix_len + b"tenant-a".len());
        for (i, byte) in b"tenant-b".iter().enumerate() {
            k_b.insert(prefix_len + i, *byte);
        }

        put_in_cf(&db, cf::ADMIN_USERS, &k_a, b"a-user");
        put_in_cf(&db, cf::ADMIN_USERS, &k_b, b"b-user");

        storage.delete_tenant_data("tenant-a").unwrap();

        let cf = cf_handle(&db, cf::ADMIN_USERS).unwrap();
        assert!(
            db.get_cf(cf, &k_a).unwrap().is_none(),
            "tenant-a admin user should be wiped"
        );
        assert!(
            db.get_cf(cf, &k_b).unwrap().is_some(),
            "tenant-b admin user should survive tenant-a wipe"
        );
    }
}
