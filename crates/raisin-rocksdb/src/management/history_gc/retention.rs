//! How much revision history a branch keeps, and where that choice is stored.
//!
//! A policy is resolved most-specific-first: the branch's own policy, then the
//! repository's (`*`), then the server default passed in [`super::GcOptions`].
//! Policies live in [`cf::REGISTRY`] under
//! `{tenant}\0history_retention\0{repo}\0{branch|*}` as JSON, beside the
//! `{tenant}\0vmounts\0…` namespace and — like it — wiped with the tenant.
//!
//! JSON rather than msgpack on purpose: the record is tiny, read once per GC
//! pass, and a field added later must not make older builds fail to read it
//! (a positional msgpack struct would).

use crate::{cf, cf_handle, keys::KeyBuilder};
use raisin_error::{Error, Result};
use rocksdb::DB;
use serde::{Deserialize, Serialize};

/// Registry namespace segment for retention policies.
const NAMESPACE: &str = "history_retention";

/// The branch segment that stands for "every branch of the repository".
pub const ALL_BRANCHES: &str = "*";

/// How much revision history to keep.
///
/// Both limits are "at least": a revision is kept when EITHER limit still
/// covers it, so `keep_days = 7, keep_revisions = 100` keeps the last 100
/// revisions even on a branch that has not been written for a month. With
/// neither set, nothing is ever pruned (the historical behaviour).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryRetention {
    /// Keep every revision newer than this many days.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_days: Option<u32>,
    /// Keep at least this many of the branch's most recent revisions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_revisions: Option<u64>,
}

impl HistoryRetention {
    /// Keep everything (no pruning).
    pub const KEEP_ALL: HistoryRetention = HistoryRetention {
        keep_days: None,
        keep_revisions: None,
    };

    /// Whether this policy prunes anything at all.
    pub fn is_keep_all(&self) -> bool {
        self.keep_days.is_none() && self.keep_revisions.is_none()
    }
}

fn policy_key(tenant_id: &str, repo_id: &str, branch: &str) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(NAMESPACE)
        .push(repo_id)
        .push(branch)
        .build()
}

/// The policy stored for exactly this repo/branch (`branch = "*"` for the
/// repository-wide one), without any fallback.
pub fn get_policy(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
) -> Result<Option<HistoryRetention>> {
    let cf = cf_handle(db, cf::REGISTRY)?;
    let Some(bytes) = db
        .get_cf(cf, policy_key(tenant_id, repo_id, branch))
        .map_err(|e| Error::storage(e.to_string()))?
    else {
        return Ok(None);
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|e| Error::storage(format!("invalid history retention policy: {e}")))
}

/// Store (or with `None`, remove) the policy for a repo/branch.
pub fn set_policy(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    policy: Option<&HistoryRetention>,
) -> Result<()> {
    let cf = cf_handle(db, cf::REGISTRY)?;
    let key = policy_key(tenant_id, repo_id, branch);
    match policy {
        Some(p) => {
            let bytes = serde_json::to_vec(p).map_err(|e| Error::storage(e.to_string()))?;
            db.put_cf(cf, key, bytes)
        }
        None => db.delete_cf(cf, key),
    }
    .map_err(|e| Error::storage(e.to_string()))
}

/// Resolve the effective policy: branch, then repository, then `default`.
pub fn resolve_policy(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    default: HistoryRetention,
) -> Result<HistoryRetention> {
    if let Some(p) = get_policy(db, tenant_id, repo_id, branch)? {
        return Ok(p);
    }
    if let Some(p) = get_policy(db, tenant_id, repo_id, ALL_BRANCHES)? {
        return Ok(p);
    }
    Ok(default)
}
