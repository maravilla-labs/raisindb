//! Who may build what, and where the engine may write.
//!
//! - **The record** the engine mirrors into is written with the engine's own
//!   privileges, so an unvalidated record path would be a write-anywhere
//!   primitive. It must be `settings:/(packages|snapshots)/<key>/<run>/state`
//!   on the build's own tenant, repository and branch, exist, be a
//!   [`RECORD_NODE_TYPE`], and have its request node as parent.
//! - **The actor** of a build started from a function is never taken from the
//!   request: it is the person whose write created the request node, read
//!   from that node's FIRST revision, whose `updated_by` the write path
//!   stamps unconditionally from the transaction's principal (a client can
//!   pre-fill `created_by`, never that). It must be a person (not the
//!   system, an agent, a trigger, a flow, a function or an API key), resolve
//!   to an identity user in this repository, and be allowed to export.
//! - **The denylist** keeps secrets and engine state out of any archive,
//!   whatever the selection compiled to: denied workspaces are refused at
//!   submit, denied node types are skipped during the walk (a node of that
//!   type can appear under a selected subtree after dispatch).
//! - **Jobs** are only visible to their own tenant AND repository.

use super::mirror::RecordRef;
use crate::RocksDBStorage;
use raisin_error::{Error, Result};
use raisin_models::auth::AuthContext;
use raisin_storage::jobs::{JobId, JobInfo, JobType};
use raisin_storage::{NodeRepository, Storage, StorageScope};
use std::sync::Arc;

/// The workspace build records live in.
pub const RECORD_WORKSPACE: &str = "settings";
/// The node type of a build record (Studio's build state).
pub const RECORD_NODE_TYPE: &str = "studio:PackageBuildState";

/// Workspaces an export never reads.
pub const DENIED_WORKSPACES: &[&str] = &[
    "raisin:system",
    "settings",
    "job_activity",
    "publish_state",
    "link_state",
    "raisin:access_control",
    "secrets",
];

/// Node types an export never archives.
pub const DENIED_NODE_TYPES: &[&str] = &[
    "raisin:Identity",
    "raisin:Session",
    "raisin:OneTimeToken",
    "raisin:ProviderConfig",
    "raisin:ConnectionConfig",
    "raisin:ConnectorConfig",
    "raisin:EmailConfig",
    "raisin:McpConnection",
    "raisin:Integration",
    "studio:EventSigningKey",
];

/// May an export read workspace `ws`?
pub fn workspace_allowed(ws: &str) -> bool {
    let lower = ws.to_ascii_lowercase();
    !(DENIED_WORKSPACES.iter().any(|d| lower == *d)
        || lower.ends_with("_jobs")
        || lower.contains("access_control")
        || ws.is_empty()
        || ws.contains('/')
        || ws.contains('\0'))
}

/// May an export archive a node of type `node_type`?
pub fn node_type_allowed(node_type: &str) -> bool {
    !DENIED_NODE_TYPES.contains(&node_type)
}

fn forbidden(msg: impl Into<String>) -> Error {
    Error::Forbidden(format!("forbidden: {}", msg.into()))
}

fn segment_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && s != "."
        && s != ".."
}

/// `/(packages|snapshots)/<key>/<run>/state`, and the request node's path.
pub fn record_shape(record: &RecordRef) -> Option<String> {
    if record.workspace != RECORD_WORKSPACE {
        return None;
    }
    let parts: Vec<&str> = record.path.split('/').collect();
    match parts.as_slice() {
        ["", area, key, run, "state"]
            if matches!(*area, "packages" | "snapshots") && segment_ok(key) && segment_ok(run) =>
        {
            Some(format!("/{area}/{key}/{run}"))
        }
        _ => None,
    }
}

/// Check a record the engine is asked to write into: shape, scope, type and
/// parent. Returns the request node's path.
pub async fn validate_record(
    storage: &Arc<RocksDBStorage>,
    tenant: &str,
    repo: &str,
    branch: &str,
    record: &RecordRef,
) -> Result<String> {
    let parent_path = record_shape(record).ok_or_else(|| {
        Error::Validation(format!(
            "The build record must be {RECORD_WORKSPACE}:/packages/<key>/<run>/state \
             (or /snapshots/…), not {}:{}",
            record.workspace, record.path
        ))
    })?;
    let scope = StorageScope::new(tenant, repo, branch, &record.workspace);
    let state = storage
        .nodes()
        .get_by_path(scope, &record.path, None)
        .await?
        .ok_or_else(|| Error::Validation("The build record does not exist".to_string()))?;
    if state.node_type != RECORD_NODE_TYPE {
        return Err(Error::Validation(format!(
            "The build record must be a {RECORD_NODE_TYPE}, not {}",
            state.node_type
        )));
    }
    if storage
        .nodes()
        .get_by_path(scope, &parent_path, None)
        .await?
        .is_none()
    {
        return Err(Error::Validation(
            "The build record has no request node".to_string(),
        ));
    }
    Ok(parent_path)
}

/// Is `actor` a person, as far as its spelling can tell?
pub fn looks_like_a_person(actor: &str) -> bool {
    let a = actor.trim();
    !(a.is_empty()
        || matches!(a, "system" | "anonymous" | "package-build" | "job-activity")
        || [
            "agent:",
            "trigger:",
            "flow:",
            "function:",
            "api_key:",
            "apikey:",
            "key:",
            "system:",
        ]
        .iter()
        .any(|p| a.starts_with(p)))
}

/// The person who created the request node, as an [`AuthContext`] with
/// their permissions in this repository — or a refusal.
pub async fn requester_from_record(
    storage: &Arc<RocksDBStorage>,
    tenant: &str,
    repo: &str,
    branch: &str,
    record: &RecordRef,
) -> Result<AuthContext> {
    let parent_path = validate_record(storage, tenant, repo, branch, record).await?;
    let scope = StorageScope::new(tenant, repo, branch, &record.workspace);
    let request = storage
        .nodes()
        .get_by_path(scope, &parent_path, None)
        .await?
        .ok_or_else(|| forbidden("the build request is gone"))?;
    let history = storage
        .nodes()
        .get_node_history(scope, &request.id, None)
        .await?;
    // Newest first: the last entry is the create.
    let creator = history
        .last()
        .and_then(|e| e.updated_by.clone())
        .ok_or_else(|| forbidden("the build request has no recorded creator"))?;
    if let Some(claimed) = request.created_by.as_deref() {
        if claimed != creator {
            return Err(forbidden(
                "the build request's created_by does not match who wrote it",
            ));
        }
    }
    if !looks_like_a_person(&creator) {
        return Err(forbidden(format!(
            "builds are requested by a person, not by '{creator}'"
        )));
    }
    let perms = raisin_core::PermissionService::new(storage.clone())
        .resolve_for_identity_id(tenant, repo, branch, &creator)
        .await?
        .ok_or_else(|| forbidden("the build request's creator is not a user of this repository"))?;
    let auth = AuthContext::for_user(&creator).with_permissions(perms);
    raisin_core::services::package_export_access::authorize_package_export(Some(&auth))
        .map_err(|_| forbidden("the build request's creator may not export packages"))?;
    Ok(auth)
}

/// The repository a package job belongs to: its result's stamp, else its
/// context.
fn job_repo(storage: &RocksDBStorage, info: &JobInfo) -> Option<String> {
    if let Some(repo) = info
        .result
        .as_ref()
        .and_then(raisin_core::services::package_export_access::result_repo)
    {
        return Some(repo.to_string());
    }
    storage
        .job_data_store()
        .get(&info.tenant, &info.id)
        .ok()
        .flatten()
        .map(|ctx| ctx.repo_id)
}

/// A package build job of exactly this tenant and repository, or `None` —
/// the same answer for a foreign job as for a missing one.
pub async fn scoped_job(
    storage: &Arc<RocksDBStorage>,
    tenant: &str,
    repo: &str,
    job_id: &str,
) -> Option<JobInfo> {
    let info = storage
        .job_registry()
        .get_job_info(&JobId(job_id.to_string()))
        .await
        .ok()?;
    if info.tenant != tenant || !matches!(info.job_type, JobType::PackageCreateFromSelection { .. })
    {
        return None;
    }
    (job_repo(storage, &info).as_deref() == Some(repo)).then_some(info)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(ws: &str, path: &str) -> RecordRef {
        RecordRef {
            workspace: ws.into(),
            path: path.into(),
        }
    }

    #[test]
    fn only_the_state_node_of_a_run_is_a_record() {
        assert_eq!(
            record_shape(&rec("settings", "/packages/site/b-1/state")).as_deref(),
            Some("/packages/site/b-1")
        );
        assert!(record_shape(&rec("settings", "/snapshots/all/b-2/state")).is_some());
        for (ws, p) in [
            ("stories", "/packages/site/b-1/state"),
            ("settings", "/packages/site/b-1"),
            ("settings", "/packages/site/b-1/state/x"),
            ("settings", "/other/site/b-1/state"),
            ("settings", "/packages/../b-1/state"),
            ("settings", "/packages/site//state"),
            ("settings", "packages/site/b-1/state"),
            ("raisin:access_control", "/packages/a/b/state"),
        ] {
            assert!(record_shape(&rec(ws, p)).is_none(), "{ws}:{p}");
        }
    }

    #[test]
    fn the_denylist() {
        for ws in [
            "raisin:system",
            "settings",
            "SETTINGS",
            "job_activity",
            "flows_jobs",
            "publish_state",
            "link_state",
            "raisin:access_control",
            "_raisin__access_control",
            "",
            "a/b",
        ] {
            assert!(!workspace_allowed(ws), "{ws}");
        }
        for ws in ["stories", "assets", "nodetypes", "people"] {
            assert!(workspace_allowed(ws), "{ws}");
        }
        assert!(!node_type_allowed("raisin:Identity"));
        assert!(!node_type_allowed("studio:EventSigningKey"));
        assert!(node_type_allowed("studio:Page"));
    }

    #[test]
    fn people_and_machines() {
        for a in ["u-123", "alice@example.com", "6f1c…"] {
            assert!(looks_like_a_person(a), "{a}");
        }
        for a in [
            "",
            "system",
            "anonymous",
            "agent:/agents/bot",
            "trigger:/triggers/t",
            "flow:/flows/f",
            "function:/lib/x",
            "api_key:abc",
            "package-build",
        ] {
            assert!(!looks_like_a_person(a), "{a}");
        }
    }
}
