//! The rebuild proper: read every `raisin:Event` in one workspace, diff the
//! projection against what the masters want, apply in one transaction.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chrono::Utc;
use raisin_core::services::node_service::NodeService;
use raisin_error::Result;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::Node;
use raisin_storage::transactional::{TransactionalContext, TransactionalStorage};

use super::job::RebuildSummary;
use super::master::{
    parse_exception, parse_master, recurrence_type, SeriesMaster, TYPE_OCCURRENCE,
};
use super::project::{matches, occurrence_node, occurrence_path};
use super::rule::{expand, ExpandError};
use super::{EVENT_TYPE, MAX_OCCURRENCES_PER_MASTER, WINDOW_FUTURE_DAYS, WINDOW_PAST_DAYS};
use crate::RocksDBStorage;

/// Actor stamped on every projection write, so `updated_by` says what wrote it.
pub const EXPANDER_ACTOR: &str = "calendar-expander";

/// The rebuild proper: one read of every `raisin:Event`, one diff, one
/// transaction.
pub(super) async fn rebuild_locked(
    storage: &Arc<RocksDBStorage>,
    tenant: &str,
    repo: &str,
    branch: &str,
    workspace: &str,
) -> Result<RebuildSummary> {
    let svc: NodeService<RocksDBStorage> = NodeService::new_with_context(
        storage.clone(),
        tenant.to_string(),
        repo.to_string(),
        branch.to_string(),
        workspace.to_string(),
    )
    .with_auth(AuthContext::system_as(EXPANDER_ACTOR));

    let events = svc.list_by_type(EVENT_TYPE).await?;
    let mut summary = RebuildSummary {
        workspaces_scanned: 1,
        ..Default::default()
    };
    if events.is_empty() {
        return Ok(summary);
    }

    let plan = plan_projection(&events, workspace, &mut summary);
    if plan.desired.is_empty() && plan.existing.is_empty() {
        return Ok(summary);
    }

    apply(storage, tenant, repo, branch, workspace, plan, &mut summary).await?;
    Ok(summary)
}

/// The diff, computed entirely in memory from nodes already read.
struct Plan {
    /// Path → the node the projection wants there.
    desired: HashMap<String, Node>,
    /// Path → existing projection node.
    existing: HashMap<String, Node>,
}

fn plan_projection(events: &[Node], workspace: &str, summary: &mut RebuildSummary) -> Plan {
    // Exception slots, keyed by the master's provider id. An exception is
    // authoritative for its instant, so the generated occurrence at that slot is
    // suppressed — a cancelled one leaves a hole (correct: the instance does not
    // happen), a rescheduled one is represented by the exception node itself,
    // which already carries its own indexed start_utc.
    let mut suppressed: HashSet<(String, String)> = HashSet::new();
    for node in events {
        if let Some(slot) = parse_exception(node) {
            suppressed.insert((slot.series_master_external_id, slot.original_start_utc));
        }
    }

    let mut existing: HashMap<String, Node> = HashMap::new();
    let mut masters: Vec<SeriesMaster> = Vec::new();
    for node in events {
        if recurrence_type(node) == TYPE_OCCURRENCE {
            existing.insert(node.path.clone(), node.clone());
            continue;
        }
        if let Some(master) = parse_master(node) {
            masters.push(master);
        }
    }
    summary.masters = masters.len();

    let now = Utc::now();
    let window_start = now - chrono::Duration::days(WINDOW_PAST_DAYS);
    let window_end = now + chrono::Duration::days(WINDOW_FUTURE_DAYS);

    let mut desired: HashMap<String, Node> = HashMap::new();
    for master in &masters {
        // A cancelled series produces nothing. Its master stays, carrying
        // status: cancelled, which is what a client needs to show "this series
        // was called off" — but there is no instance to project.
        if master
            .status
            .as_deref()
            .is_some_and(|s| s.eq_ignore_ascii_case("cancelled"))
        {
            continue;
        }
        let expansion = match expand(master, window_start, window_end, MAX_OCCURRENCES_PER_MASTER) {
            Ok(e) => e,
            Err(ExpandError::NoRule) => continue,
            Err(e) => {
                summary.failed_masters += 1;
                tracing::warn!(
                    master = %master.node_id, error = %e,
                    "calendar-expand: master could not be expanded; skipping it"
                );
                continue;
            }
        };
        if expansion.truncated {
            summary.truncated_masters += 1;
            tracing::warn!(
                master = %master.node_id,
                cap = MAX_OCCURRENCES_PER_MASTER,
                "calendar-expand: recurrence hit the per-master cap; the window is not \
                 fully covered by this series"
            );
        }
        for occ in expansion.occurrences {
            let slot = super::format_utc(occ.start_utc);
            if let Some(ext) = &master.external_id {
                if suppressed.contains(&(ext.clone(), slot)) {
                    summary.suppressed += 1;
                    continue;
                }
            }
            let path = occurrence_path(&master.node_id, occ.start_utc);
            // Keep the existing node's id so the projection updates in place.
            let id = existing
                .get(&path)
                .map(|n| n.id.clone())
                .unwrap_or_else(|| nanoid::nanoid!());
            desired.insert(path, occurrence_node(id, workspace, master, &occ));
        }
    }

    // Anything belonging to a master that no longer exists is pruned by the
    // same diff: `existing` holds every projection node in the workspace,
    // `desired` only what the surviving masters want.
    Plan { desired, existing }
}

async fn apply(
    storage: &Arc<RocksDBStorage>,
    tenant: &str,
    repo: &str,
    branch: &str,
    workspace: &str,
    plan: Plan,
    summary: &mut RebuildSummary,
) -> Result<()> {
    let mut writes: Vec<Node> = Vec::new();
    for (path, node) in &plan.desired {
        match plan.existing.get(path) {
            // Already exactly what the projection wants: writing it would mint a
            // revision per occurrence every 15 minutes, forever.
            Some(current) if matches(current, &node.properties) => summary.unchanged += 1,
            _ => writes.push(node.clone()),
        }
    }
    let prunes: Vec<&Node> = plan
        .existing
        .iter()
        .filter(|(path, _)| !plan.desired.contains_key(*path))
        .map(|(_, node)| node)
        .collect();

    if writes.is_empty() && prunes.is_empty() {
        return Ok(());
    }

    let tx = begin(storage, tenant, repo, branch).await?;
    for node in &writes {
        // Deep upsert auto-creates the `/_occurrences/{master}/{YYYY}/{MM}`
        // folders. Missing them is not an option: a node written to a path whose
        // parents do not exist is unreachable by any hierarchy query.
        tx.upsert_deep_node(workspace, node, "raisin:Folder")
            .await?;
    }
    for node in &prunes {
        tx.delete_node(workspace, &node.id).await?;
    }
    tx.commit().await?;

    summary.written += writes.len();
    summary.pruned += prunes.len();
    Ok(())
}

async fn begin(
    storage: &Arc<RocksDBStorage>,
    tenant: &str,
    repo: &str,
    branch: &str,
) -> Result<Box<dyn TransactionalContext>> {
    let tx = storage.begin_context().await?;
    tx.set_tenant_repo(tenant, repo)?;
    tx.set_branch(branch)?;
    // Both halves carry the expander: the raw actor lands on `RevisionMeta`, the
    // auth context is what stamps `created_by`/`updated_by` — and the auth
    // context wins there, which is the exact trap the sync actor fix closed.
    tx.set_actor(EXPANDER_ACTOR)?;
    tx.set_auth_context(AuthContext::system_as(EXPANDER_ACTOR))?;
    tx.set_message("calendar occurrence projection")?;
    Ok(tx)
}
