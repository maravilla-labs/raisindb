//! Package builds, for the HTTP endpoint and the function binding (E1).
//!
//! The submit, cancel and status entry points live with the job
//! (`jobs/handlers/package_create_from_selection/submit.rs`); this module
//! re-exports them and adds what a function needs on top: the process's
//! storage (installed once by the server, the same slot pattern as
//! `node_dev`) and one JSON dispatch for `raisin.packages.*`.

use crate::RocksDBStorage;
use raisin_error::{Error, Result};
use raisin_models::auth::AuthContext;
use serde_json::{json, Value};
use std::sync::{Arc, OnceLock, RwLock, Weak};

pub use crate::jobs::{
    cancel_package_build, package_build_status, submit_package_build, BuildStatus,
    PackageBuildRequest, RecordRefWire, SelectedPath, Submitted,
};

static SLOT: OnceLock<RwLock<Weak<RocksDBStorage>>> = OnceLock::new();

fn slot() -> &'static RwLock<Weak<RocksDBStorage>> {
    SLOT.get_or_init(|| RwLock::new(Weak::new()))
}

/// Make this process's storage reachable from `raisin.packages.*`. Held
/// weakly: the storage's lifetime stays the server's.
pub fn install(storage: &Arc<RocksDBStorage>) {
    *slot().write().expect("package-build slot poisoned") = Arc::downgrade(storage);
}

fn installed() -> Option<Arc<RocksDBStorage>> {
    slot()
        .read()
        .expect("package-build slot poisoned")
        .upgrade()
}

/// Where a call runs: the calling function's repository and branch.
pub struct CallScope<'a> {
    pub tenant: &'a str,
    pub repo: &'a str,
    pub branch: &'a str,
}

/// The capabilities `packages_capabilities` reports. Studio gates its
/// Packages UI on these flags, never on a version compare.
pub fn capabilities() -> Value {
    json!({
        "streaming_build": true,
        "record_mirror": true,
        "sidecars": true,
        "ranged_download": true,
        "cancel": true,
        "idempotency": true,
        "excluded_paths": false,
        "version": env!("CARGO_PKG_VERSION"),
    })
}

/// The largest range `resources_readRange` returns in one call.
pub const MAX_READ_RANGE: u64 = 8 * 1024 * 1024;

/// Accept Studio's request shape (`package_name`, `workspace_selections:
/// [{workspace, selected_paths, excluded_paths?}]`, `options`) as well as
/// the HTTP one (`name`, `selected_paths: [{workspace, path}]`).
fn normalize_request(mut args: Value) -> Result<PackageBuildRequest> {
    let invalid = |m: String| Error::Validation(format!("Invalid package build request: {m}"));
    let obj = args
        .as_object_mut()
        .ok_or_else(|| invalid("expected an object".into()))?;
    if let Some(name) = obj.remove("package_name") {
        obj.entry("name").or_insert(name);
    }
    if let Some(sel) = obj.remove("workspace_selections") {
        let sel = sel
            .as_array()
            .ok_or_else(|| invalid("workspace_selections must be an array".into()))?;
        let mut paths = Vec::new();
        for ws in sel {
            let workspace = ws
                .get("workspace")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid("a workspace selection names no workspace".into()))?;
            if ws
                .get("excluded_paths")
                .and_then(Value::as_array)
                .is_some_and(|a| !a.is_empty())
            {
                return Err(Error::Validation(
                    "engine_unsupported: excluded_paths (capability excluded_paths is false)"
                        .into(),
                ));
            }
            for p in ws
                .get("selected_paths")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let path = p
                    .as_str()
                    .ok_or_else(|| invalid("selected_paths must be strings".into()))?;
                paths.push(json!({ "workspace": workspace, "path": path }));
            }
        }
        obj.insert("selected_paths".into(), Value::Array(paths));
    }
    // Options (DESIGN §4.5) are accepted and not yet honoured beyond
    // include_node_types; display-only fields are dropped.
    obj.remove("options");
    obj.remove("requested_by");
    serde_json::from_value(args).map_err(|e| invalid(e.to_string()))
}

/// The function bindings `raisin.packages.*` / `raisin.resources.readRange`:
///
/// - `capabilities` → [`capabilities`].
/// - `createFromSelection(request)` → `{job_id, existing}`. `record`
///   (settings:/packages/<key>/<run>/state) is REQUIRED; the build's actor is
///   the person who created the record's request node — never the
///   function's identity nor anything in the request.
/// - `cancel(jobId)` → `{job_id, cancelled}`; cooperative, ≤ 5 s then abort.
/// - `status(jobId)` → `{job_id, status, progress, error, result}` | null.
/// - `deleteArtifacts({workspace, path})` → `{deleted}`: the blobs the
///   engine's ledger records for that build state node, nothing else.
/// - `readRange({storage_key, offset, len})` → `{base64, bytes}`: only a
///   key the ledger records as an artifact of this tenant and repository,
///   at most [`MAX_READ_RANGE`] bytes.
///
/// Everything is scoped to the calling function's tenant, repository and
/// branch; a foreign job, record or key reads as unknown.
pub async fn call(
    scope: CallScope<'_>,
    auth: &AuthContext,
    method: &str,
    args: Value,
) -> Result<Value> {
    if method == "capabilities" {
        return Ok(capabilities());
    }
    let storage = installed().ok_or_else(|| {
        Error::Validation(
            "engine_unsupported: package builds are not running on this server".into(),
        )
    })?;
    use crate::jobs::handlers::package_create_from_selection::{ledger, scope as build_scope};
    match method {
        "createFromSelection" => {
            if args.get("branch").is_some() {
                return Err(Error::Validation(
                    "a package build runs on the calling function's branch; 'branch' is not accepted"
                        .into(),
                ));
            }
            let request = normalize_request(args)?;
            let record: crate::jobs::handlers::package_create_from_selection::RecordRef = request
                .record
                .clone()
                .ok_or_else(|| Error::Validation("'record' is required".into()))?
                .into();
            let requester = build_scope::requester_from_record(
                &storage,
                scope.tenant,
                scope.repo,
                scope.branch,
                &record,
            )
            .await?;
            let submitted = submit_package_build(
                &storage,
                scope.tenant,
                scope.repo,
                scope.branch,
                &request,
                &requester,
            )
            .await?;
            Ok(json!({
                "job_id": submitted.job_id,
                "existing": submitted.existing,
                "status": "scheduled",
            }))
        }
        "cancel" => {
            let job_id = job_id_of(&args)?;
            authorize(auth)?;
            let cancelled =
                cancel_package_build(&storage, scope.tenant, scope.repo, &job_id).await?;
            Ok(json!({ "job_id": job_id, "cancelled": cancelled }))
        }
        "status" => {
            let job_id = job_id_of(&args)?;
            authorize(auth)?;
            Ok(
                match package_build_status(&storage, scope.tenant, scope.repo, &job_id).await? {
                    Some(status) => serde_json::to_value(status).unwrap_or(Value::Null),
                    None => Value::Null,
                },
            )
        }
        "deleteArtifacts" => {
            authorize(auth)?;
            let record: crate::jobs::handlers::package_create_from_selection::RecordRef =
                serde_json::from_value(args)
                    .map_err(|e| Error::Validation(format!("expected {{workspace, path}}: {e}")))?;
            if build_scope::record_shape(&record).is_none() {
                return Err(Error::Validation(
                    "deleteArtifacts takes a build state node (settings:/packages/<key>/<run>/state)"
                        .into(),
                ));
            }
            let store = storage
                .blob_store()
                .ok_or_else(|| Error::Validation("engine_unsupported: no blob store".into()))?;
            let deleted = ledger::delete_for(
                &storage,
                store.as_ref(),
                scope.tenant,
                scope.repo,
                ledger::Owner::Path {
                    branch: scope.branch,
                    workspace: &record.workspace,
                    path: &record.path,
                },
            )
            .await?;
            Ok(json!({ "deleted": deleted }))
        }
        "readRange" => {
            authorize(auth)?;
            read_range(&storage, scope, &args).await
        }
        other => Err(Error::Validation(format!(
            "raisin.packages has no method '{other}'"
        ))),
    }
}

async fn read_range(
    storage: &Arc<RocksDBStorage>,
    scope: CallScope<'_>,
    args: &Value,
) -> Result<Value> {
    use crate::jobs::handlers::package_create_from_selection::ledger;
    use futures::StreamExt;
    let key = args
        .get("storage_key")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Validation("'storage_key' is required".into()))?;
    let offset = args.get("offset").and_then(Value::as_u64).unwrap_or(0);
    let len = args
        .get("len")
        .and_then(Value::as_u64)
        .unwrap_or(MAX_READ_RANGE)
        .min(MAX_READ_RANGE);
    let not_found = || Error::NotFound(format!("No package artifact '{key}'"));
    // Only what the engine itself recorded as an artifact of this repository.
    let owner = ledger::lookup(storage, scope.tenant, scope.repo, key)?.ok_or_else(not_found)?;
    if owner.branch != scope.branch
        || !raisin_core::is_artifact_key_of(scope.tenant, scope.repo, key)
    {
        return Err(not_found());
    }
    let store = storage
        .blob_store()
        .ok_or_else(|| Error::Validation("engine_unsupported: no blob store".into()))?;
    let size = store
        .head(key)
        .await
        .map_err(|e| Error::storage(e.to_string()))?
        .ok_or_else(not_found)?
        .size;
    let start = offset.min(size);
    let end = start.saturating_add(len).min(size);
    let mut stream = store
        .get_range(key, start..end)
        .await
        .map_err(|e| Error::storage(e.to_string()))?;
    let mut buf = Vec::with_capacity((end - start) as usize);
    while let Some(chunk) = stream.next().await {
        buf.extend_from_slice(&chunk.map_err(|e| Error::storage(e.to_string()))?);
    }
    use base64::Engine as _;
    Ok(json!({
        "base64": base64::engine::general_purpose::STANDARD.encode(&buf),
        "bytes": buf.len(),
        "offset": start,
        "size": size,
        "eof": end >= size,
    }))
}

fn authorize(auth: &AuthContext) -> Result<()> {
    raisin_core::services::package_export_access::authorize_package_export(Some(auth))
        .map_err(|_| Error::Forbidden("forbidden: package builds require an administrator".into()))
}

fn job_id_of(args: &Value) -> Result<String> {
    args.as_str()
        .or_else(|| args.get("jobId").and_then(Value::as_str))
        .or_else(|| args.get("job_id").and_then(Value::as_str))
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| Error::Validation("a job id is required".into()))
}
