//! The streaming build of a branch's localized name index
//! (`RepairKind::LocalizedNames`), on the repair machinery: bounded batches,
//! a resumable cursor committed with them, a disk precheck, a rate limit, a
//! per-node state record and the admin fan-out (`management::async_indexing::repair`).
//!
//! **Pinned.** A fresh run pins the branch HEAD (`built_from_rev`) and the
//! repository's fingerprint, kept in the repair state across a resume; a
//! resumed run under another fingerprint starts over. Every node is read as of
//! the pin and its segments are put at the revision of the newest record of
//! the node at or below the pin — its version, ANY overlay version (deletes
//! included) and any existing index row (a catch-up, a merge's full put, a
//! checkpoint peer's) — so the build's rows are the newest the node has up to
//! the pin and nothing older can outrank them at HEAD. Writes ABOVE the pin
//! are the inline writers' — they run whether or not a build is `Ready` — so
//! from the pin on the index is complete, and reads below the pin take the
//! fallback.
//!
//! **Data-detected.** A node whose reverse rows already say what the selector
//! says is not rewritten; a run over a maintained index writes nothing.
//!
//! **Collisions.** After the node pass, each workspace's claims are grouped by
//! `(locale, parent, name)` and a group with two or more VERIFIED claims is a
//! collision. The count lands in the state record (uniqueness enforcement
//! waits for zero) and the first few in the report, for the console.
//!
//! **Ready.** Each workspace's record went `Building` under a fresh
//! generation when the run started (kept in the resumable epoch with the pin
//! and the fingerprint); at the end it becomes `Ready` only if it is still
//! `Building` under this run's fingerprint, pin and generation
//! (`state::finish_build`).
//!
//! **Refused while the index is switched off** (`RAISIN_LOCALIZED_NAME_INDEX`):
//! with the inline writers off, a `Ready` stamp would outlive every write
//! they skipped.

use super::config;
use super::keys::NameScope;
use super::state;
use crate::management::async_indexing::node_key_parse::parse_node_key;
use crate::management::async_indexing::repair::BoundedWriter;
use crate::{cf, cf_handle};
use raisin_error::Result;
use raisin_hlc::HLC;
use rocksdb::{ReadOptions, DB};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

mod collisions;
mod node_rows;

pub(crate) use node_rows::claim_verifies;
use node_rows::node_rows;

const PASS: &str = "localized_names";

/// What a build found on one branch.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalizedNameCounts {
    /// Live nodes read at the pin.
    pub nodes: u64,
    /// Nodes whose rows were (or, in a dry run, would be) rewritten.
    pub rewritten: u64,
    /// Workspaces stamped `Ready`.
    pub ready_workspaces: u64,
    /// Sibling collisions, all workspaces.
    pub collisions: u64,
    /// The first few collisions: `ws/locale/parent/name: node, node…`.
    #[serde(default)]
    pub collision_samples: Vec<String>,
    /// The pinned revision, as text.
    #[serde(default)]
    pub built_from_rev: Option<String>,
}

/// Every workspace of the repository (its `WORKSPACES` records).
pub(crate) fn list_workspaces(db: &DB, tenant_id: &str, repo_id: &str) -> Result<Vec<String>> {
    let prefix = crate::keys::KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push("workspaces")
        .build_prefix();
    let cf = cf_handle(db, cf::WORKSPACES)?;
    let mut out = Vec::new();
    for item in crate::prefix_scan(db, cf, &prefix) {
        let (key, _) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;
        if let Ok(name) = std::str::from_utf8(&key[prefix.len()..]) {
            if !name.is_empty() && !name.contains('\0') {
                out.push(name.to_string());
            }
        }
    }
    Ok(out)
}

/// The resumable epoch: `{pin}|{fingerprint}|{generation}`.
fn parse_epoch(epoch: Option<&str>) -> Option<(HLC, String, u64)> {
    let mut parts = epoch?.splitn(3, '|');
    let pin = parts.next()?.parse().ok()?;
    let fingerprint = parts.next()?.to_string();
    let generation = parts.next()?.parse().ok()?;
    Some((pin, fingerprint, generation))
}

/// Build one branch. Returns `false` when the run stopped early (resumable).
pub(crate) fn rebuild_branch(
    db: &DB,
    (tenant_id, repo_id, branch): (&str, &str, &str),
    writer: &mut BoundedWriter<'_>,
    resumed: bool,
    counts: &mut LocalizedNameCounts,
) -> Result<bool> {
    if !super::enabled() {
        return Err(raisin_error::Error::Validation(format!(
            "the localized name index is switched off ({}); not building it",
            super::LOCALIZED_NAME_INDEX_ENV
        )));
    }
    let Some(cfg) = config::load(db, tenant_id, repo_id)? else {
        return Ok(true);
    };
    let fingerprint = cfg.fingerprint();
    let (pin, generation) = match parse_epoch(writer.state().epoch.as_deref()) {
        Some((pin, fp, generation)) if resumed && fp == fingerprint => (pin, generation),
        _ => {
            writer.clear_cursor();
            let Some(head) = crate::management::async_indexing::repair::branch_head(
                db, tenant_id, repo_id, branch,
            )?
            else {
                return Ok(true);
            };
            let generation = if writer.dry_run() {
                0
            } else {
                let mut workspaces = list_workspaces(db, tenant_id, repo_id)?;
                workspaces.sort();
                state::begin_build(
                    db,
                    (tenant_id, repo_id, branch),
                    &workspaces,
                    &fingerprint,
                    &head,
                )?
            };
            (head, generation)
        }
    };
    writer.set_epoch(Some(format!("{pin}|{fingerprint}|{generation}")));
    counts.built_from_rev = Some(pin.to_string());
    writer.begin_pass(PASS);

    if !node_pass(db, (tenant_id, repo_id, branch), writer, &cfg, &pin, counts)? {
        return Ok(false);
    }
    let found = collisions::count(db, (tenant_id, repo_id, branch), &cfg, &pin, counts)?;
    if !writer.dry_run() {
        // The node pass's last writes must be durable before `Ready`.
        writer.commit("running")?;
        let stamped = state::finish_build(
            db,
            (tenant_id, repo_id, branch),
            &fingerprint,
            &pin,
            generation,
            &found,
        )?;
        counts.ready_workspaces = stamped.len() as u64;
    }
    Ok(true)
}

/// Stream the branch's NODES; for each node's newest version at or before
/// the pin, put its segments.
fn node_pass(
    db: &DB,
    (tenant_id, repo_id, branch): (&str, &str, &str),
    writer: &mut BoundedWriter<'_>,
    cfg: &config::NameConfig,
    pin: &HLC,
    counts: &mut LocalizedNameCounts,
) -> Result<bool> {
    let branch_prefix = crate::keys::branch_prefix(tenant_id, repo_id, branch);
    let cursor = writer
        .state()
        .cursor
        .as_deref()
        .and_then(|h| hex::decode(h).ok());
    let mut opts = ReadOptions::default();
    opts.set_total_order_seek(true);
    opts.fill_cache(false);
    if let Some(upper) = crate::prefix_successor(&branch_prefix) {
        opts.set_iterate_upper_bound(upper);
    }
    let mut iter = db.raw_iterator_cf_opt(cf_handle(db, cf::NODES)?, opts);
    match cursor.as_deref().map(crate::prefix_successor) {
        Some(Some(next)) => iter.seek(&next),
        Some(None) => return Ok(true),
        None => iter.seek(&branch_prefix),
    }
    let mut parents: BTreeMap<(String, String), Option<String>> = BTreeMap::new();
    let mut done_group: Option<Vec<u8>> = None;
    while iter.valid() {
        let Some(key) = iter.key() else { break };
        let Some((workspace, node_id, revision)) = parse_node_key(&branch_prefix, key) else {
            iter.next();
            continue;
        };
        let group = key[..key.len() - 16].to_vec();
        // Versions run newest first: the first at or below the pin decides.
        if done_group.as_deref() == Some(group.as_slice()) || revision > *pin {
            iter.next();
            continue;
        }
        let (workspace, node_id) = (workspace.to_string(), node_id.to_string());
        done_group = Some(group);
        let scope = NameScope::new(tenant_id, repo_id, branch, &workspace);
        if let Some(rows) = node_rows(db, scope, &node_id, cfg, pin, &mut parents, counts)? {
            for (key, value) in rows {
                writer.put(cf::LOCALIZED_NAME_INDEX, &key, &value)?;
            }
        }
        let group_key =
            crate::keys::node_key_prefix(tenant_id, repo_id, branch, &workspace, &node_id);
        if !writer.checkpoint(PASS, &group_key)? {
            return Ok(false);
        }
        if parents.len() > 100_000 {
            parents.clear();
        }
        iter.next();
    }
    iter.status()
        .map_err(|e| raisin_error::Error::storage(e.to_string()))?;
    Ok(true)
}
