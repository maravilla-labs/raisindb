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

/// `raisin.packages.<method>(args)`:
///
/// - `createFromSelection(request)` — the HTTP request body
///   ([`PackageBuildRequest`]), with `record` REQUIRED. Returns
///   `{job_id, existing, status: "scheduled"}`.
/// - `cancel({jobId})` — cooperative cancel (≤ 5 s, then abort). Returns
///   `{job_id, cancelled}`; `cancelled: false` for an unknown or foreign job.
/// - `status({jobId})` — `{job_id, status, progress, error, result}` or
///   `null` for an unknown or foreign job.
///
/// Everything is scoped to the calling function's tenant, repository and
/// branch; nothing in `args` can widen it. The build's actor is never the
/// function's identity nor anything in the request: it is the person who
/// created the record's request node (see `scope::requester_from_record`).
pub async fn call(
    scope: CallScope<'_>,
    auth: &AuthContext,
    method: &str,
    args: Value,
) -> Result<Value> {
    let storage = installed().ok_or_else(|| {
        Error::Validation(
            "engine_unsupported: package builds are not running on this server".into(),
        )
    })?;
    match method {
        "createFromSelection" => {
            if args.get("branch").is_some() {
                return Err(Error::Validation(
                    "a package build runs on the calling function's branch; 'branch' is not accepted"
                        .into(),
                ));
            }
            let request: PackageBuildRequest = serde_json::from_value(args)
                .map_err(|e| Error::Validation(format!("Invalid package build request: {e}")))?;
            let record: crate::jobs::handlers::package_create_from_selection::RecordRef = request
                .record
                .clone()
                .ok_or_else(|| Error::Validation("'record' is required".into()))?
                .into();
            let requester =
                crate::jobs::handlers::package_create_from_selection::scope::requester_from_record(
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
        other => Err(Error::Validation(format!(
            "raisin.packages has no method '{other}' (createFromSelection, cancel, status)"
        ))),
    }
}

fn authorize(auth: &AuthContext) -> Result<()> {
    raisin_core::services::package_export_access::authorize_package_export(Some(auth))
        .map_err(|_| Error::Forbidden("forbidden: package builds require an administrator".into()))
}

fn job_id_of(args: &Value) -> Result<String> {
    args.get("jobId")
        .or_else(|| args.get("job_id"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| Error::Validation("'jobId' is required".into()))
}
