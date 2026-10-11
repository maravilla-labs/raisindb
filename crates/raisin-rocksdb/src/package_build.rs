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

/// Who is calling a binding, as the SERVER knows it — never read from args.
#[derive(Debug, Clone)]
pub struct Invoker {
    /// The calling execution's own identity.
    pub auth: AuthContext,
    /// Set only for a genuine trigger run: the execution runs as the system
    /// under a `trigger:` agent marker (written by trigger evaluation, which
    /// also built the run's input), and this is the node whose event fired it.
    pub trigger_event: Option<TriggerEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerEvent {
    pub node_id: String,
    /// `Created`, `Updated`, …
    pub kind: String,
}

impl Invoker {
    /// A trigger run, if the identity and the event both say so.
    pub fn from_execution(auth: AuthContext, event: Option<&Value>) -> Self {
        let is_trigger = auth.is_system
            && auth
                .agent
                .as_deref()
                .and_then(|a| a.split('@').next())
                .is_some_and(|head| head.starts_with("trigger:"));
        let trigger_event = if is_trigger {
            event.and_then(|e| {
                Some(TriggerEvent {
                    node_id: e.get("node_id")?.as_str()?.to_string(),
                    kind: e.get("type")?.as_str()?.to_string(),
                })
            })
        } else {
            None
        };
        Self {
            auth,
            trigger_event,
        }
    }

    /// The calling person, if the execution runs as one.
    fn person(&self) -> Option<&str> {
        let a = &self.auth;
        if a.is_system || a.is_anonymous_principal() {
            return None;
        }
        a.user_id.as_deref().filter(|u| {
            crate::jobs::handlers::package_create_from_selection::scope::looks_like_a_person(u)
        })
    }
}

type Record = crate::jobs::handlers::package_create_from_selection::RecordRef;

/// The `/(packages|snapshots)/<key>` a build state path belongs to.
fn definition_of(path: &str) -> Option<String> {
    let parts: Vec<&str> = path.split('/').collect();
    match parts.as_slice() {
        ["", area, key, ..] if matches!(*area, "packages" | "snapshots") && !key.is_empty() => {
            Some(format!("/{area}/{key}"))
        }
        _ => None,
    }
}

/// May `invoker` act on the build whose state node is `record`?
///
/// - a trigger run whose event node lies under the SAME package definition
///   (`settings:/packages/<key>/…`) as the record — the dispatch, cancel and
///   finalize triggers Studio configures for that definition; or
/// - a person who may export (raisin_core's export grant) AND can read the
///   record's state node under their own row-level security.
///
/// The system, an API key, or a function with no such binding is refused.
async fn entitled(
    storage: &Arc<RocksDBStorage>,
    scope: &CallScope<'_>,
    invoker: &Invoker,
    record: &Record,
) -> Result<()> {
    use raisin_storage::{NodeRepository, Storage, StorageScope};
    let forbidden = || {
        Error::Forbidden("forbidden: this execution may not act on that package build".to_string())
    };
    let ws = StorageScope::new(scope.tenant, scope.repo, scope.branch, &record.workspace);
    if let Some(event) = &invoker.trigger_event {
        let target_def = definition_of(&record.path).ok_or_else(forbidden)?;
        let fired_by = storage
            .nodes()
            .get(ws, &event.node_id, None)
            .await?
            .ok_or_else(forbidden)?;
        if definition_of(&fired_by.path).as_deref() == Some(target_def.as_str()) {
            return Ok(());
        }
        return Err(forbidden());
    }
    if invoker.person().is_some() {
        raisin_core::services::package_export_access::authorize_package_export(Some(&invoker.auth))
            .map_err(|_| forbidden())?;
        let state = storage
            .nodes()
            .get_by_path(ws, &record.path, None)
            .await?
            .ok_or_else(forbidden)?;
        if raisin_core::services::package_export_access::readable_by_requester(
            state,
            &invoker.auth,
            &record.workspace,
            scope.branch,
        )
        .is_some()
        {
            return Ok(());
        }
    }
    Err(forbidden())
}

/// The record a job was built for: its context (while it runs) or the stamp
/// on its result (after).
fn record_of_job(
    storage: &RocksDBStorage,
    tenant: &str,
    job: &raisin_storage::jobs::JobInfo,
) -> Option<Record> {
    let from = |v: &Value| serde_json::from_value::<Record>(v.clone()).ok();
    if let Some(r) = job
        .result
        .as_ref()
        .and_then(|r| r.get("record"))
        .and_then(from)
    {
        return Some(r);
    }
    storage
        .job_data_store()
        .get(tenant, &job.id)
        .ok()
        .flatten()
        .and_then(|ctx| ctx.metadata.get("record").and_then(from))
}

/// A stable digest of what a dispatch asks for, so a second dispatch of the
/// same request can be told apart from a different one.
fn request_digest(request: &PackageBuildRequest) -> String {
    use sha2::{Digest, Sha256};
    let mut v = serde_json::to_value(request).unwrap_or(Value::Null);
    if let Some(o) = v.as_object_mut() {
        o.remove("idempotency_key");
    }
    hex::encode(Sha256::digest(v.to_string().as_bytes()))
}

/// The function bindings `raisin.packages.*` / `raisin.resources.readRange`:
///
/// - `capabilities` → [`capabilities`].
/// - `createFromSelection(request)` → `{job_id, existing}`. `record`
///   (settings:/packages/<key>/<run>/state) is REQUIRED. Only the trigger
///   fired by that run's request or state node — or the very person who
///   created the request — may dispatch it; the build's actor is that
///   person (from the server's stamps), never the function. A run is
///   dispatched once: a repeat with the same request returns the same job,
///   a repeat with a different request is refused.
/// - `cancel(jobId)` → `{job_id, cancelled}`; cooperative, ≤ 5 s then abort.
/// - `status(jobId)` → `{job_id, status, progress, error, result}` | null.
/// - `deleteArtifacts({workspace, path})` → `{deleted}`: the blobs the
///   engine's ledger records for that build state node, nothing else.
/// - `readRange({storage_key, offset, len})` → `{base64, bytes}`: only a
///   key the ledger records as an artifact of this tenant and repository,
///   at most [`MAX_READ_RANGE`] bytes.
///
/// Everything after `capabilities` runs [`entitled`] for the build it
/// touches, and is scoped to the calling function's tenant, repository and
/// branch; a foreign job, record or key reads as unknown.
pub async fn call(
    scope: CallScope<'_>,
    invoker: &Invoker,
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
    call_with(&storage, scope, invoker, method, args).await
}

/// [`call`] against an explicit storage (embedders and tests).
pub async fn call_with(
    storage: &Arc<RocksDBStorage>,
    scope: CallScope<'_>,
    invoker: &Invoker,
    method: &str,
    args: Value,
) -> Result<Value> {
    if method == "capabilities" {
        return Ok(capabilities());
    }
    let storage = storage.clone();
    use crate::jobs::handlers::package_create_from_selection::{ledger, scope as build_scope};
    match method {
        "createFromSelection" => {
            if args.get("branch").is_some() {
                return Err(Error::Validation(
                    "a package build runs on the calling function's branch; 'branch' is not accepted"
                        .into(),
                ));
            }
            let mut request = normalize_request(args)?;
            let record: Record = request
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
            // The deputy check: only the trigger fired by THIS run's request
            // or state node, or the requester in person, may dispatch it.
            dispatch_entitled(&storage, &scope, invoker, &record, &requester).await?;
            // One dispatch per run, keyed by the record — never by a caller's
            // key — and bound to what was asked the first time.
            request.idempotency_key = Some(format!("{}:{}", record.workspace, record.path));
            let digest = request_digest(&request);
            if let Some(existing) = already_dispatched(&storage, &scope, &record).await? {
                return match existing {
                    (job_id, Some(d)) if d == digest => Ok(json!({
                        "job_id": job_id, "existing": true, "status": "scheduled",
                    })),
                    _ => Err(Error::Conflict(
                        "this package build was already dispatched with other parameters".into(),
                    )),
                };
            }
            let submitted =
                crate::jobs::handlers::package_create_from_selection::submit::submit_with_digest(
                    &storage,
                    scope.tenant,
                    scope.repo,
                    scope.branch,
                    &request,
                    &requester,
                    &digest,
                )
                .await?;
            Ok(json!({
                "job_id": submitted.job_id,
                "existing": submitted.existing,
                "status": "scheduled",
            }))
        }
        "cancel" | "status" => {
            let job_id = job_id_of(&args)?;
            let Some(job) =
                build_scope::scoped_job(&storage, scope.tenant, scope.repo, &job_id).await
            else {
                return Ok(if method == "cancel" {
                    json!({ "job_id": job_id, "cancelled": false })
                } else {
                    Value::Null
                });
            };
            let record = record_of_job(&storage, scope.tenant, &job).ok_or_else(|| {
                Error::Forbidden(
                    "forbidden: that job has no build record to authorize against".into(),
                )
            })?;
            entitled(&storage, &scope, invoker, &record).await?;
            if method == "cancel" {
                let cancelled =
                    cancel_package_build(&storage, scope.tenant, scope.repo, &job_id).await?;
                Ok(json!({ "job_id": job_id, "cancelled": cancelled }))
            } else {
                Ok(
                    match package_build_status(&storage, scope.tenant, scope.repo, &job_id).await? {
                        Some(status) => serde_json::to_value(status).unwrap_or(Value::Null),
                        None => Value::Null,
                    },
                )
            }
        }
        "deleteArtifacts" => {
            let record: Record = serde_json::from_value(args)
                .map_err(|e| Error::Validation(format!("expected {{workspace, path}}: {e}")))?;
            if build_scope::record_shape(&record).is_none() {
                return Err(Error::Validation(
                    "deleteArtifacts takes a build state node (settings:/packages/<key>/<run>/state)"
                        .into(),
                ));
            }
            entitled(&storage, &scope, invoker, &record).await?;
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
        "readRange" => read_range(&storage, scope, invoker, &args).await,
        other => Err(Error::Validation(format!(
            "raisin.packages has no method '{other}'"
        ))),
    }
}

/// createFromSelection's stricter binding: the trigger fired by THIS run's
/// request or state node (not merely the same definition), or the requester.
async fn dispatch_entitled(
    storage: &Arc<RocksDBStorage>,
    scope: &CallScope<'_>,
    invoker: &Invoker,
    record: &Record,
    requester: &AuthContext,
) -> Result<()> {
    use raisin_storage::{NodeRepository, Storage, StorageScope};
    let forbidden = || {
        Error::Forbidden(
            "forbidden: only the trigger fired by this build request, or its requester, may dispatch it"
                .to_string(),
        )
    };
    if let Some(event) = &invoker.trigger_event {
        let ws = StorageScope::new(scope.tenant, scope.repo, scope.branch, &record.workspace);
        let request_path = build_scope_parent(record).ok_or_else(forbidden)?;
        for path in [record.path.as_str(), request_path.as_str()] {
            if let Some(n) = storage.nodes().get_by_path(ws, path, None).await? {
                if n.id == event.node_id {
                    return Ok(());
                }
            }
        }
        return Err(forbidden());
    }
    if let Some(person) = invoker.person() {
        if Some(person) == requester.user_id.as_deref() {
            return Ok(());
        }
    }
    Err(forbidden())
}

fn build_scope_parent(record: &Record) -> Option<String> {
    crate::jobs::handlers::package_create_from_selection::scope::record_shape(record)
}

/// `(job_id, digest)` if this run was already dispatched: the state node
/// carries a job id (the mirror stamps it at the build's first write; Studio
/// stamps it after dispatch), and the digest of that dispatch if still known.
async fn already_dispatched(
    storage: &Arc<RocksDBStorage>,
    scope: &CallScope<'_>,
    record: &Record,
) -> Result<Option<(String, Option<String>)>> {
    use raisin_models::nodes::properties::PropertyValue;
    use raisin_storage::{NodeRepository, Storage, StorageScope};
    let ws = StorageScope::new(scope.tenant, scope.repo, scope.branch, &record.workspace);
    // A dispatch still in the registry under this record's key.
    let key = format!("{}:{}", record.workspace, record.path);
    let from_registry = crate::jobs::handlers::package_create_from_selection::submit::job_for_key(
        storage,
        scope.tenant,
        scope.repo,
        &key,
    )
    .await;
    let stamped = match storage.nodes().get_by_path(ws, &record.path, None).await? {
        Some(n) => match n.properties.get("job_id") {
            Some(PropertyValue::String(s)) if !s.is_empty() => Some(s.clone()),
            _ => None,
        },
        None => None,
    };
    let Some(job_id) = from_registry.clone().or(stamped) else {
        return Ok(None);
    };
    let digest = crate::jobs::handlers::package_create_from_selection::submit::job_digest(
        storage,
        scope.tenant,
        &job_id,
    )
    .await;
    Ok(Some((job_id, digest)))
}

async fn read_range(
    storage: &Arc<RocksDBStorage>,
    scope: CallScope<'_>,
    invoker: &Invoker,
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
    // Reading an archive is exporting it: the same entitlement as the build.
    let record = Record {
        workspace: owner.workspace.clone(),
        path: owner.path.clone(),
    };
    entitled(storage, &scope, invoker, &record).await?;
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

fn job_id_of(args: &Value) -> Result<String> {
    args.as_str()
        .or_else(|| args.get("jobId").and_then(Value::as_str))
        .or_else(|| args.get("job_id").and_then(Value::as_str))
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| Error::Validation("a job id is required".into()))
}
