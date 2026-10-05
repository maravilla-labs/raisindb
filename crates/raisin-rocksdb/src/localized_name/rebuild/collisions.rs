//! The collision pass of a build: per workspace, claims grouped by
//! `(locale, parent, name)`; a group with two or more claims that VERIFY at
//! the pin is one collision.

use super::{claim_verifies, list_workspaces, LocalizedNameCounts};
use crate::localized_name::config::NameConfig;
use crate::localized_name::keys::{self, NameScope};
use crate::{cf, cf_handle};
use raisin_error::Result;
use raisin_hlc::HLC;
use rocksdb::DB;
use std::collections::BTreeMap;

const SAMPLES: usize = 20;

/// Count collisions per workspace (and record samples in `counts`).
pub(super) fn count(
    db: &DB,
    (tenant_id, repo_id, branch): (&str, &str, &str),
    cfg: &NameConfig,
    pin: &HLC,
    counts: &mut LocalizedNameCounts,
) -> Result<BTreeMap<String, u64>> {
    let mut per_workspace = BTreeMap::new();
    for workspace in list_workspaces(db, tenant_id, repo_id)? {
        let scope = NameScope::new(tenant_id, repo_id, branch, &workspace);
        let found = count_workspace(db, scope, cfg, pin, counts)?;
        counts.collisions += found;
        per_workspace.insert(workspace, found);
    }
    Ok(per_workspace)
}

fn count_workspace(
    db: &DB,
    scope: NameScope<'_>,
    cfg: &NameConfig,
    pin: &HLC,
    counts: &mut LocalizedNameCounts,
) -> Result<u64> {
    let prefix = crate::keys::KeyBuilder::new()
        .push(scope.tenant_id)
        .push(scope.repo_id)
        .push(scope.branch)
        .push(scope.workspace)
        .push("lname")
        .build_prefix();
    let cf = cf_handle(db, cf::LOCALIZED_NAME_INDEX)?;
    let mut collisions = 0;
    // The group being collected: its (locale, parent, name) and, per node,
    // the newest claim at or below the pin.
    let mut group: Option<(String, String, String)> = None;
    let mut members: BTreeMap<String, keys::ForwardKey> = BTreeMap::new();
    let mut decided: Vec<String> = Vec::new();
    let mut flush = |members: &mut BTreeMap<String, keys::ForwardKey>| -> Result<u64> {
        if members.len() < 2 {
            members.clear();
            return Ok(0);
        }
        let mut verified = Vec::new();
        for claim in members.values() {
            if claim_verifies(db, scope, claim, cfg, pin)? {
                verified.push(claim.node_id.clone());
            }
        }
        let found = members.values().next().map(|c| {
            format!(
                "{}/{}/{}/{}: {}",
                c.workspace,
                c.locale,
                c.parent_id,
                c.name,
                verified.join(", ")
            )
        });
        members.clear();
        if verified.len() < 2 {
            return Ok(0);
        }
        if let Some(sample) = found {
            if counts.collision_samples.len() < SAMPLES {
                counts.collision_samples.push(sample);
            }
        }
        Ok(1)
    };
    for item in crate::prefix_scan(db, cf, &prefix) {
        let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;
        let Some(claim) = keys::parse_forward_key(&key) else {
            continue;
        };
        let this = (
            claim.locale.clone(),
            claim.parent_id.clone(),
            claim.name.clone(),
        );
        if group.as_ref() != Some(&this) {
            collisions += flush(&mut members)?;
            decided.clear();
            group = Some(this);
        }
        // Newest first per node: the first version at or below the pin decides.
        if claim.revision > *pin || decided.contains(&claim.node_id) {
            continue;
        }
        decided.push(claim.node_id.clone());
        if !crate::keys::is_tombstone_value(&value) {
            members.insert(claim.node_id.clone(), claim);
        }
    }
    collisions += flush(&mut members)?;
    Ok(collisions)
}
