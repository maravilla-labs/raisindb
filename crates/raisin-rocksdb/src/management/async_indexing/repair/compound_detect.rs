//! What the `compound_builds` repair owes a branch, read from the DATA (plan
//! Phase 13f): the workspace records, the compound state records, and one
//! seek per workspace index name in the compound keyspace. No NodeType read,
//! so it is cheap enough for the chain's pending check.

use super::RepairKind;
use crate::compound_state::CompoundStateStore;
use crate::{cf, cf_handle};
use raisin_error::{Error, Result};
use raisin_models::nodes::properties::schema::CompoundIndexDefinition;
use raisin_models::workspace::builtin_indexes::is_builtin_stored_name;
use raisin_storage::compound::{CompoundBuildPhase, CompoundIndexState, CompoundStateSource};
use rocksdb::DB;

/// One unit of the repair's work on a branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Work {
    /// Build `index` of `workspace`. `owner` is `workspace:{ws}` for a
    /// workspace index, `None` for a node-type index whose declaring type the
    /// link resolves.
    Build {
        workspace: String,
        index: String,
        owner: Option<String>,
    },
    /// Drop `index` of `workspace` (record and entries): a workspace index
    /// no longer declared — removed, or a built-in index switched off.
    Drop { workspace: String, index: String },
}

impl Work {
    /// `{workspace}\0{index}`, the order the link works in and its cursor.
    pub(crate) fn key(&self) -> String {
        match self {
            Work::Build {
                workspace, index, ..
            }
            | Work::Drop { workspace, index } => format!("{workspace}\0{index}"),
        }
    }
}

/// The workspaces of a repository with a record, by name.
pub(crate) fn workspace_names(db: &DB, tenant_id: &str, repo_id: &str) -> Result<Vec<String>> {
    let cf = cf_handle(db, cf::WORKSPACES)?;
    let prefix = crate::keys::KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push("workspaces")
        .build_prefix();
    let mut out = Vec::new();
    for item in crate::prefix_scan(db, cf, &prefix) {
        let (key, _) = item.map_err(|e| Error::storage(e.to_string()))?;
        if !key.starts_with(&prefix) {
            break;
        }
        let name = String::from_utf8_lossy(&key[prefix.len()..]);
        let name = name.trim_end_matches('\0');
        if !name.is_empty() && !name.contains('\0') {
            out.push(name.to_string());
        }
    }
    Ok(out)
}

/// The WORKSPACE index names (`@…`) with at least one entry in `workspace`'s
/// compound keyspace on `branch`, either publication tag: one seek per name,
/// skipping the rest of its entries.
fn workspace_index_names_with_entries(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
) -> Result<Vec<String>> {
    use raisin_models::nodes::properties::schema::WORKSPACE_INDEX_PREFIX;
    let cf = cf_handle(db, cf::COMPOUND_INDEX)?;
    let mut names = std::collections::BTreeSet::new();
    let mut iter = db.raw_iterator_cf(cf);
    for published in [false, true] {
        let base = crate::keys::compound_index_workspace_prefix(
            tenant_id, repo_id, branch, workspace, published,
        );
        let mut seek = [base.as_slice(), WORKSPACE_INDEX_PREFIX.as_bytes()].concat();
        loop {
            iter.seek(&seek);
            let Some(key) = iter.key() else {
                break;
            };
            let Some(rest) = key.strip_prefix(base.as_slice()) else {
                break;
            };
            if !rest.starts_with(WORKSPACE_INDEX_PREFIX.as_bytes()) {
                break;
            }
            let Some(end) = rest.iter().position(|b| *b == 0) else {
                break;
            };
            let name = &rest[..end];
            names.insert(String::from_utf8_lossy(name).into_owned());
            // Past every entry of this name: `{base}{name}\x01`.
            seek = [base.as_slice(), name, &[1u8]].concat();
        }
        iter.status()
            .map_err(|e| Error::storage(format!("compound keyspace scan failed: {e}")))?;
    }
    Ok(names.into_iter().collect())
}

/// Every compound state record on a branch: `(workspace, index, state)`.
pub(crate) fn branch_records(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
) -> Result<Vec<(String, String, CompoundIndexState)>> {
    let cf = cf_handle(db, cf::INDEX_STATUS)?;
    let prefix = format!("compound_index\0{tenant_id}\0{repo_id}\0{branch}\0").into_bytes();
    let mut out = Vec::new();
    for item in crate::prefix_scan(db, cf, &prefix) {
        let (key, value) = item.map_err(|e| Error::storage(e.to_string()))?;
        if !key.starts_with(&prefix) {
            break;
        }
        let rest = String::from_utf8_lossy(&key[prefix.len()..]).into_owned();
        let mut parts = rest.splitn(2, '\0');
        let (Some(ws), Some(name)) = (parts.next(), parts.next()) else {
            continue;
        };
        if let Ok(state) = rmp_serde::from_slice::<CompoundIndexState>(&value) {
            out.push((ws.to_string(), name.to_string(), state));
        }
    }
    Ok(out)
}

/// The work the data says this branch owes. `all_unready` also lists every
/// other declared-or-recorded index that is not `Ready` (the link's safety net
/// after a checkpoint ingest marked every record); without it only what this
/// repair OWNS: unbuilt built-in indexes, older-format records (while the
/// automatic format rebuild is on) and undeclared workspace indexes.
pub(crate) fn branch_work(
    db: &std::sync::Arc<DB>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    all_unready: bool,
) -> Result<Vec<Work>> {
    let store = CompoundStateStore::new(db.clone());
    let mut work: Vec<Work> = Vec::new();
    let mut declared_by: std::collections::HashMap<String, Vec<CompoundIndexDefinition>> =
        std::collections::HashMap::new();
    for workspace in workspace_names(db, tenant_id, repo_id)? {
        let declared = match crate::indexing::compound::workspace_defs::stored(
            db, tenant_id, repo_id, &workspace,
        ) {
            Ok(declared) => declared,
            Err(e) => {
                // Unknown declarations: neither build nor drop anything there.
                tracing::warn!(workspace = %workspace, error = %e, "compound_builds: workspace record unreadable; skipped");
                continue;
            }
        };
        for definition in &declared {
            let ours = is_builtin_stored_name(&definition.name);
            if (ours || all_unready)
                && !store
                    .compound_availability(tenant_id, repo_id, branch, &workspace, definition)
                    .is_ready()
            {
                work.push(Work::Build {
                    workspace: workspace.clone(),
                    index: definition.name.clone(),
                    owner: Some(format!("workspace:{workspace}")),
                });
            }
        }
        // Entries of a workspace index nothing declares, and no record says
        // so: maintained by the writers while it was declared but switched off
        // before any build registered here (or after a refused precheck).
        for index in workspace_index_names_with_entries(db, tenant_id, repo_id, branch, &workspace)?
        {
            if !declared.iter().any(|d| d.name == index) {
                work.push(Work::Drop {
                    workspace: workspace.clone(),
                    index,
                });
            }
        }
        declared_by.insert(workspace, declared);
    }
    let format_rebuild = crate::compound_state::format_rebuild_enabled();
    for (workspace, index, state) in branch_records(db, tenant_id, repo_id, branch)? {
        let Some(declared) = declared_by.get(&workspace) else {
            continue; // no workspace record: leave its records alone
        };
        if CompoundIndexDefinition::is_workspace_index_name(&index) {
            if !declared.iter().any(|d| d.name == index) {
                work.push(Work::Drop { workspace, index });
            }
            // A declared one was judged above.
            continue;
        }
        // An older-format record is owed only while the automatic format
        // rebuild is on — whatever its phase; switched off, it is an admin's.
        let owed = if state.is_format_upgrade() {
            format_rebuild
        } else {
            all_unready && state.phase != CompoundBuildPhase::Ready
        };
        if owed {
            work.push(Work::Build {
                workspace,
                index,
                owner: None,
            });
        }
    }
    work.sort_by_key(Work::key);
    work.dedup_by(|a, b| a.key() == b.key());
    Ok(work)
}

/// Every `(tenant, repo, branch)` the chain still owes: this node's state
/// record is not `done`, or the data shows owed work (in order).
pub fn pending_branches(storage: &crate::RocksDBStorage) -> Result<Vec<(String, String, String)>> {
    let db = storage.db();
    let node_id = super::repair_node_id(storage);
    let mut out = Vec::new();
    for (tenant_id, repo_id) in super::list_repositories(db)? {
        for branch in super::list_branches(db, &tenant_id, &repo_id)? {
            if branch_pending(db, &tenant_id, &repo_id, &branch, &node_id)? {
                out.push((tenant_id.clone(), repo_id.clone(), branch));
            }
        }
    }
    out.sort();
    Ok(out)
}

/// A stable fingerprint of `work` (kind, workspace and index of each item),
/// stored on a `failed` link's state record: a TARGETED request skips a
/// failed branch while the work it owes is still exactly this — the failure
/// (unplaceable nodes, no headroom, …) is retried at the next start, not on
/// every workspace event and cold drain — and runs again as soon as the work
/// changed (an index switched off, a workspace added).
pub(crate) fn work_fingerprint(work: &[Work]) -> String {
    let items: Vec<String> = work
        .iter()
        .map(|item| {
            let kind = match item {
                Work::Build { .. } => "build",
                Work::Drop { .. } => "drop",
            };
            format!("{kind}:{}", item.key().replace('\0', "/"))
        })
        .collect();
    format!("work[{}]", items.join(","))
}

/// Whether one branch is pending (see [`pending_branches`]).
pub(crate) fn branch_pending(
    db: &std::sync::Arc<DB>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    node_id: &str,
) -> Result<bool> {
    let slug = RepairKind::CompoundBuilds.slug();
    let state = super::load_state(db, tenant_id, repo_id, branch, slug, node_id)?;
    if state.is_none_or(|s| s.status != "done") {
        return Ok(true);
    }
    Ok(!branch_work(db, tenant_id, repo_id, branch, false)?.is_empty())
}
