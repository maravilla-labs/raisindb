//! Merge-base pins for retention GC (plan Phase 9, "merge-base pin").
//!
//! A fork point (`created_from`) is the merge base only until the first merge.
//! After it, the divergence walk (`branches/divergence.rs`) follows BOTH
//! parents of every merge commit, so the highest common ancestor of the next
//! merge is the earlier merge commit itself, or the source head it merged in
//! (its `merge_parent`) — and the three-way merge reads both branches AT that
//! revision. Without a pin, retention GC prunes the versions those reads
//! resolve to, and the second merge sees a base where the nodes are absent.

use crate::{cf, cf_handle, keys};
use raisin_error::{Error, Result};
use raisin_hlc::HLC;
use rocksdb::DB;

/// The distinct `(tenant, repo)` pairs of the branch records.
pub(super) fn repositories(
    branches: &[(String, String, raisin_context::Branch)],
) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = branches
        .iter()
        .map(|(t, r, _)| (t.clone(), r.clone()))
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Every merge commit of a repository and its second parent.
pub(super) fn merge_revisions(db: &DB, tenant: &str, repo: &str) -> Result<Vec<HLC>> {
    let cf = cf_handle(db, cf::REVISIONS)?;
    let prefix = keys::KeyBuilder::new()
        .push(tenant)
        .push(repo)
        .push("revisions")
        .build_prefix();
    let mut read_opts = rocksdb::ReadOptions::default();
    read_opts.fill_cache(false);
    let mut it = db.raw_iterator_cf_opt(cf, read_opts);
    it.seek(&prefix);
    let mut out = Vec::new();
    while it.valid() {
        let (Some(key), Some(value)) = (it.key(), it.value()) else {
            break;
        };
        if !key.starts_with(&prefix) {
            break;
        }
        if let Ok(meta) = rmp_serde::from_slice::<raisin_storage::RevisionMeta>(value) {
            if let Some(second) = meta.merge_parent {
                out.push(meta.revision);
                out.push(second);
            }
        }
        it.next();
    }
    it.status().map_err(|e| Error::storage(e.to_string()))?;
    Ok(out)
}
