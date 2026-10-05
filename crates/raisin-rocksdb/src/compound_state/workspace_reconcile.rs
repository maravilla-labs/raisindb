//! Reconciling WORKSPACE-owned compound state records with the workspace's
//! current declarations (plan Phase 13e; called by
//! `indexing::compound::workspace_defs` whenever the record changed).
//!
//! A NodeType declaration change is noticed by the NodeType write and by the
//! definitions-cache refresh (`refresh.rs`). A workspace record has more
//! writers (repository, replication, transactions, backup import, checkpoint
//! ingest), so the check runs where the declarations are READ instead: every
//! writer of an entry reads them first, so no entry is written under a new
//! declaration before this has run.

use super::marker::{mark, transitions};
use crate::{cf, cf_handle};
use raisin_error::{Error, Result};
use raisin_models::nodes::properties::schema::CompoundIndexDefinition;
use raisin_storage::compound::{CompoundBuildPhase, CompoundIndexState};
use rocksdb::{WriteBatch, DB};

/// Mark `NotBuilt` every workspace-index record of `workspace` (any branch)
/// whose phase is not already `NotBuilt` and that `declared` does not vouch
/// for — no declaration of that name, or one with a different hash. A
/// `Building` record of the CURRENT declaration is a build of the right
/// thing and is left alone. Returns the branches where a still-declared index
/// was marked (they need a build).
pub fn reconcile_workspace_declarations(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    workspace: &str,
    declared: &[CompoundIndexDefinition],
) -> Result<Vec<String>> {
    let _guard = transitions();
    let cf = cf_handle(db, cf::INDEX_STATUS)?;
    let prefix = format!("compound_index\0{tenant_id}\0{repo_id}\0").into_bytes();
    let mut batch = WriteBatch::default();
    let mut rebuild: Vec<String> = Vec::new();
    for item in crate::prefix_scan(db, cf, &prefix) {
        let (key, value) =
            item.map_err(|e| Error::storage(format!("compound state scan failed: {e}")))?;
        if !key.starts_with(&prefix) {
            break;
        }
        // Remainder: `{branch}\0{workspace}\0{index_name}`.
        let rest = String::from_utf8_lossy(&key[prefix.len()..]).into_owned();
        let mut parts = rest.splitn(3, '\0');
        let (Some(branch), Some(ws), Some(name)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        if ws != workspace || !CompoundIndexDefinition::is_workspace_index_name(name) {
            continue;
        }
        let Ok(mut state) = rmp_serde::from_slice::<CompoundIndexState>(&value) else {
            continue; // unreadable: reads as not built already
        };
        if state.phase == CompoundBuildPhase::NotBuilt {
            continue;
        }
        let current = declared.iter().find(|d| d.name == name);
        if current.is_some_and(|d| d.definition_hash() == state.definition_hash) {
            continue;
        }
        mark(&mut state);
        let bytes = rmp_serde::to_vec(&state)
            .map_err(|e| Error::storage(format!("Failed to serialize compound state: {e}")))?;
        batch.put_cf(cf, &key, bytes);
        tracing::info!(
            index = %name,
            workspace = %workspace,
            branch = %branch,
            declared = current.is_some(),
            "workspace compound declaration changed or removed; index marked NotBuilt"
        );
        if current.is_some() && !rebuild.iter().any(|b| b == branch) {
            rebuild.push(branch.to_string());
        }
    }
    if !batch.is_empty() {
        db.write(batch)
            .map_err(|e| Error::storage(format!("Failed to mark compound state: {e}")))?;
    }
    Ok(rebuild)
}

/// Whether `built` (a build's result) is still what its owner declares —
/// always for a NodeType index (the definitions refresh marks its changes),
/// and for a workspace index only when the STORED workspace record declares
/// that name with the same hash. Read raw, never through the reconciling
/// reader: the caller holds the transition lock that reader would take.
pub(super) fn still_declared(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    workspace: &str,
    built: &CompoundIndexState,
) -> Result<bool> {
    if !CompoundIndexDefinition::is_workspace_index_name(&built.index_name) {
        return Ok(true);
    }
    let declared =
        crate::indexing::compound::workspace_defs::stored(db, tenant_id, repo_id, workspace)?;
    Ok(declared.iter().any(|definition| {
        definition.name == built.index_name && definition.definition_hash() == built.definition_hash
    }))
}

/// Delete the state record of WORKSPACE index `index_name` on `branch` when
/// the stored workspace record no longer declares it (removed, or a built-in
/// index the config switched off — plan Phase 13f). Under the transition
/// lock, re-checking the declaration there, so a declaration that returned
/// meanwhile keeps its record. Returns whether the record was deleted (the
/// caller, holding the keyspace lock, then clears the entries).
pub fn forget_undeclared_workspace_index(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    index_name: &str,
) -> Result<bool> {
    if !CompoundIndexDefinition::is_workspace_index_name(index_name) {
        return Ok(false);
    }
    let _guard = transitions();
    let declared =
        crate::indexing::compound::workspace_defs::stored(db, tenant_id, repo_id, workspace)?;
    if declared.iter().any(|d| d.name == index_name) {
        return Ok(false);
    }
    let cf = cf_handle(db, cf::INDEX_STATUS)?;
    let key = super::compound_state_key(tenant_id, repo_id, branch, workspace, index_name);
    db.delete_cf(cf, key)
        .map_err(|e| Error::storage(format!("Failed to delete compound state: {e}")))?;
    Ok(true)
}
