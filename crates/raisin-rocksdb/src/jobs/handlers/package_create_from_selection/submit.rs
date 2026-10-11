//! Submitting, cancelling and querying a package build — the one path the
//! HTTP endpoint and the function binding (E1) share.

use super::mirror::RecordRef;
use super::request::{KEY_RECORD, KEY_REGISTER_PACKAGE, KEY_SELECTED_PATHS};
use super::types::SelectedPath;
use crate::RocksDBStorage;
use raisin_core::package_registration;
use raisin_core::services::package_export_access;
use raisin_error::{Error, Result};
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::jobs::{JobContext, JobId, JobStatus, JobType};
use raisin_storage::{NodeRepository, Storage, StorageScope};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

/// How long a cooperative cancel may take before the task is aborted.
pub const CANCEL_GRACE: Duration = Duration::from_secs(5);

/// A request to build a package from a selection.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct PackageBuildRequest {
    pub name: String,
    #[serde(default = "default_version")]
    pub version: String,
    pub selected_paths: Vec<SelectedPath>,
    #[serde(default)]
    pub include_node_types: bool,
    /// The record to mirror progress and result onto (E2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record: Option<RecordRefWire>,
    /// Write `packages:/<name>` (legacy). Builds for Studio pass `false` (E4).
    #[serde(default = "default_true")]
    pub register_package: bool,
    /// A repeated submit with the same key returns the same job (E13).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct RecordRefWire {
    pub workspace: String,
    pub path: String,
}

fn default_version() -> String {
    "1.0.0".to_string()
}

fn default_true() -> bool {
    true
}

/// The outcome of a submit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Submitted {
    pub job_id: String,
    /// `true` when an earlier submit with the same idempotency key already
    /// created this job, and nothing new was registered.
    pub existing: bool,
}

/// A refused submit, with a code from the build-record vocabulary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub code: &'static str,
    pub message: String,
}

impl From<Refusal> for Error {
    fn from(r: Refusal) -> Self {
        match r.code {
            "forbidden" => Error::Forbidden(r.message),
            _ => Error::Validation(format!("{}: {}", r.code, r.message)),
        }
    }
}

/// Is `name` a valid package id?
pub fn name_is_valid(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// A `packages:/<name>` node that an export must never shadow: one that is
/// installed, or shipped with the server.
pub fn name_is_reserved(existing: &raisin_models::nodes::Node) -> bool {
    package_registration::package_is_installed(existing)
        || matches!(
            existing.properties.get("builtin"),
            Some(PropertyValue::Boolean(true))
        )
}

fn dedup_key(tenant: &str, repo: &str, key: &str) -> String {
    format!("pkgbuild\u{0}{tenant}\u{0}{repo}\u{0}{key}")
}

/// Validate, then register a package build job running as `requester`.
///
/// The requester must already have passed
/// [`package_export_access::authorize_package_export`]; the job reads the
/// selection under the requester's permissions and refuses to run without
/// them.
pub async fn submit_package_build(
    storage: &Arc<RocksDBStorage>,
    tenant: &str,
    repo: &str,
    branch: &str,
    request: &PackageBuildRequest,
    requester: &AuthContext,
) -> Result<Submitted> {
    submit_inner(storage, tenant, repo, branch, request, requester, None).await
}

/// [`submit_package_build`], recording `digest` (what the dispatch asked for)
/// on the job, so a later dispatch of the same run can be compared with it.
pub async fn submit_with_digest(
    storage: &Arc<RocksDBStorage>,
    tenant: &str,
    repo: &str,
    branch: &str,
    request: &PackageBuildRequest,
    requester: &AuthContext,
    digest: &str,
) -> Result<Submitted> {
    submit_inner(
        storage,
        tenant,
        repo,
        branch,
        request,
        requester,
        Some(digest),
    )
    .await
}

/// Metadata / result key: the digest of the dispatch.
pub const KEY_REQUEST_DIGEST: &str = "request_digest";

/// The job registered under idempotency `key` in this tenant/repo, if the
/// registry still holds it.
pub async fn job_for_key(
    storage: &Arc<RocksDBStorage>,
    tenant: &str,
    repo: &str,
    key: &str,
) -> Option<String> {
    storage
        .job_registry()
        .job_for_dedup_key(&dedup_key(tenant, repo, key))
        .await
        .map(|id| id.to_string())
}

/// The dispatch digest of a job: its context while it runs, its result after.
pub async fn job_digest(
    storage: &Arc<RocksDBStorage>,
    tenant: &str,
    job_id: &str,
) -> Option<String> {
    let id = JobId(job_id.to_string());
    if let Ok(info) = storage.job_registry().get_job_info(&id).await {
        if let Some(d) = info
            .result
            .as_ref()
            .and_then(|r| r.get(KEY_REQUEST_DIGEST))
            .and_then(|v| v.as_str())
        {
            return Some(d.to_string());
        }
    }
    storage
        .job_data_store()
        .get(tenant, &id)
        .ok()
        .flatten()
        .and_then(|ctx| {
            ctx.metadata
                .get(KEY_REQUEST_DIGEST)
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
}

#[allow(clippy::too_many_arguments)]
async fn submit_inner(
    storage: &Arc<RocksDBStorage>,
    tenant: &str,
    repo: &str,
    branch: &str,
    request: &PackageBuildRequest,
    requester: &AuthContext,
    digest: Option<&str>,
) -> Result<Submitted> {
    package_export_access::authorize_package_export(Some(requester)).map_err(|r| {
        Error::from(Refusal {
            code: "forbidden",
            message: format!("{r:?}"),
        })
    })?;
    if !name_is_valid(&request.name) {
        return Err(Refusal {
            code: "name_invalid",
            message: "Package name must be 1–128 letters, digits, hyphens or underscores"
                .to_string(),
        }
        .into());
    }
    if request.selected_paths.is_empty() {
        return Err(Error::Validation(
            "At least one path must be selected".to_string(),
        ));
    }
    for root in super::request::selection_roots(&request.selected_paths)? {
        if !super::scope::workspace_allowed(&root.workspace) {
            return Err(Error::Forbidden(format!(
                "forbidden: workspace '{}' can never be exported",
                root.workspace
            )));
        }
    }
    // The engine writes the record with its own privileges: it must be a
    // build state node of THIS tenant, repository and branch.
    if let Some(r) = &request.record {
        super::scope::validate_record(storage, tenant, repo, branch, &r.clone().into()).await?;
    }

    let registry = storage.job_registry();
    let dedup = request
        .idempotency_key
        .as_deref()
        .filter(|k| !k.is_empty())
        .map(|k| dedup_key(tenant, repo, k));
    if let Some(key) = &dedup {
        if let Some(existing) = registry.job_for_dedup_key(key).await {
            return Ok(Submitted {
                job_id: existing.to_string(),
                existing: true,
            });
        }
    }

    // E4: never shadow an installed or shipped package.
    let packages = StorageScope::new(
        tenant,
        repo,
        branch,
        package_registration::PACKAGES_WORKSPACE,
    );
    if let Some(existing) = storage
        .nodes()
        .get_by_path(
            packages,
            &package_registration::package_node_path(&request.name),
            None,
        )
        .await?
    {
        if name_is_reserved(&existing) {
            return Err(Refusal {
                code: "name_invalid",
                message: format!(
                    "'{}' is the name of an installed package; choose another",
                    request.name
                ),
            }
            .into());
        }
    }

    let mut metadata: HashMap<String, serde_json::Value> = HashMap::new();
    metadata.insert(
        KEY_SELECTED_PATHS.to_string(),
        serde_json::to_value(&request.selected_paths)
            .map_err(|e| Error::Validation(format!("Invalid selection: {e}")))?,
    );
    metadata.insert(
        KEY_REGISTER_PACKAGE.to_string(),
        serde_json::Value::Bool(request.register_package),
    );
    if let Some(r) = &request.record {
        metadata.insert(
            KEY_RECORD.to_string(),
            serde_json::json!({"workspace": r.workspace, "path": r.path}),
        );
    }
    for (k, v) in [
        ("package_name", Some(&request.name)),
        ("package_version", Some(&request.version)),
        ("title", request.title.as_ref()),
        ("description", request.description.as_ref()),
        ("author", request.author.as_ref()),
    ] {
        if let Some(v) = v {
            metadata.insert(k.to_string(), serde_json::json!(v));
        }
    }
    metadata.extend(package_export_access::requester_metadata(requester));
    if let Some(d) = digest {
        metadata.insert(KEY_REQUEST_DIGEST.to_string(), serde_json::json!(d));
    }

    let job_type = JobType::PackageCreateFromSelection {
        package_name: request.name.clone(),
        package_version: request.version.clone(),
        include_node_types: request.include_node_types,
    };
    let context = JobContext {
        tenant_id: tenant.to_string(),
        repo_id: repo.to_string(),
        branch: branch.to_string(),
        workspace_id: package_registration::PACKAGES_WORKSPACE.to_string(),
        revision: raisin_hlc::HLC::now(),
        metadata,
    };

    // Context first, so a worker can never claim the job without it.
    let job_id = JobId::new();
    storage.job_data_store().put(&job_id, &context)?;
    let registered = match &dedup {
        Some(key) => {
            registry
                .register_job_with_id_idempotent(
                    job_id.clone(),
                    job_type,
                    tenant.to_string(),
                    key.clone(),
                    // A build is never retried: a second attempt would race
                    // the first's record writes, and every failure it can
                    // hit (space, permissions, cancel) fails again the same.
                    Some(0),
                )
                .await?
        }
        None => {
            registry
                .register_job_with_id(
                    job_id.clone(),
                    job_type,
                    tenant.to_string(),
                    None,
                    Some(Arc::new(tokio_util::sync::CancellationToken::new())),
                    Some(0),
                )
                .await?;
            true
        }
    };
    if !registered {
        // A concurrent submit with the same key won the race.
        let _ = storage.job_data_store().delete(tenant, &job_id);
        if let Some(existing) = match &dedup {
            Some(key) => registry.job_for_dedup_key(key).await,
            None => None,
        } {
            return Ok(Submitted {
                job_id: existing.to_string(),
                existing: true,
            });
        }
        return Err(Error::Conflict(
            "A build with this idempotency key is already running".to_string(),
        ));
    }
    Ok(Submitted {
        job_id: job_id.to_string(),
        existing: false,
    })
}

/// A build job's state, as a function sees it.
#[derive(Debug, Clone, Serialize)]
pub struct BuildStatus {
    pub job_id: String,
    /// `scheduled | running | completed | failed | cancelled`
    pub status: &'static str,
    pub progress: Option<f32>,
    pub error: Option<String>,
    pub result: Option<serde_json::Value>,
}

/// Look up a package build job of this tenant and repository. Any other job
/// — another tenant's or repository's, or not a package build — reads as not
/// found.
pub async fn package_build_status(
    storage: &Arc<RocksDBStorage>,
    tenant: &str,
    repo: &str,
    job_id: &str,
) -> Result<Option<BuildStatus>> {
    let Some(info) = super::scope::scoped_job(storage, tenant, repo, job_id).await else {
        return Ok(None);
    };
    let (status, error) = match &info.status {
        JobStatus::Scheduled => ("scheduled", None),
        JobStatus::Completed => ("completed", None),
        JobStatus::Cancelled => ("cancelled", None),
        JobStatus::Failed(e) => ("failed", Some(e.clone())),
        _ => ("running", None),
    };
    Ok(Some(BuildStatus {
        job_id: job_id.to_string(),
        status,
        progress: info.progress,
        error,
        result: info.result.clone(),
    }))
}

/// Cancel a package build of this tenant and repository: cooperatively first
/// — the build stops at its next chunk, aborts its writers (no partial
/// object), writes `cancelled` onto its record — and by aborting the task if
/// it has not stopped within [`CANCEL_GRACE`]. Returns `false` for an unknown
/// or foreign job, `true` once it is no longer running.
pub async fn cancel_package_build(
    storage: &Arc<RocksDBStorage>,
    tenant: &str,
    repo: &str,
    job_id: &str,
) -> Result<bool> {
    let registry = storage.job_registry();
    let Some(info) = super::scope::scoped_job(storage, tenant, repo, job_id).await else {
        return Ok(false);
    };
    let id = info.id.clone();
    if matches!(
        info.status,
        JobStatus::Completed | JobStatus::Failed(_) | JobStatus::Cancelled
    ) {
        return Ok(true);
    }
    if let Some(token) = registry.get_cancel_token(&id).await {
        token.cancel();
        let deadline = tokio::time::Instant::now() + CANCEL_GRACE;
        while tokio::time::Instant::now() < deadline {
            match registry.get_status(&id).await {
                Ok(JobStatus::Scheduled) => break,
                Ok(JobStatus::Running) | Ok(JobStatus::Executing) => {}
                _ => return Ok(true),
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    // Never started, or did not stop in time: the hard way. Dropping the
    // task drops its writers, whose guards remove the partial object.
    registry.cancel_job(&id).await?;
    Ok(true)
}

impl From<RecordRefWire> for RecordRef {
    fn from(r: RecordRefWire) -> Self {
        RecordRef {
            workspace: r.workspace,
            path: r.path,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert!(name_is_valid("export-website_1"));
        assert!(!name_is_valid(""));
        assert!(!name_is_valid("a b"));
        assert!(!name_is_valid("../x"));
        assert!(!name_is_valid(&"x".repeat(129)));
    }

    #[test]
    fn installed_and_builtin_names_are_reserved() {
        let mut n = raisin_models::nodes::Node::default();
        assert!(!name_is_reserved(&n));
        n.properties
            .insert("builtin".into(), PropertyValue::Boolean(true));
        assert!(name_is_reserved(&n));
        let mut m = raisin_models::nodes::Node::default();
        m.properties
            .insert("installed".into(), PropertyValue::Boolean(true));
        assert!(name_is_reserved(&m));
    }

    #[test]
    fn the_wire_request_defaults_to_the_legacy_shape() {
        let r: PackageBuildRequest = serde_json::from_value(serde_json::json!({
            "name": "x",
            "selected_paths": [{"workspace": "stories", "path": "/a/*"}]
        }))
        .unwrap();
        assert_eq!(r.version, "1.0.0");
        assert!(r.register_package);
        assert!(r.record.is_none());
        assert!(r.idempotency_key.is_none());
    }
}
