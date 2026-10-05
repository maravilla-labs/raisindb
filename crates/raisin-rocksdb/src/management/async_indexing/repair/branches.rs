//! Branch enumeration for the repairs: which branches a run covers, and a
//! branch's HEAD.

use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use rocksdb::DB;

/// Every branch of a repository, from the BRANCHES records.
pub(crate) fn list_branches(db: &DB, tenant_id: &str, repo_id: &str) -> Result<Vec<String>> {
    let prefix = keys::KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push("branches")
        .build_prefix();
    let cf = cf_handle(db, cf::BRANCHES)?;
    let mut branches = Vec::new();
    for item in crate::prefix_scan(db, cf, &prefix) {
        let (key, _) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;
        if let Ok(name) = std::str::from_utf8(&key[prefix.len()..]) {
            if !name.is_empty() && !name.contains('\0') {
                branches.push(name.to_string());
            }
        }
    }
    Ok(branches)
}

/// The branch HEAD, or `None` when the record is missing.
pub(crate) fn branch_head(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
) -> Result<Option<HLC>> {
    let cf = cf_handle(db, cf::BRANCHES)?;
    let bytes = db
        .get_cf(cf, keys::branch_key(tenant_id, repo_id, branch))
        .map_err(|e| raisin_error::Error::storage(e.to_string()))?;
    match bytes {
        Some(bytes) => rmp_serde::from_slice::<raisin_context::Branch>(&bytes)
            .map(|b| Some(b.head))
            .map_err(|e| raisin_error::Error::storage(format!("Branch decode error: {e}"))),
        None => Ok(None),
    }
}
