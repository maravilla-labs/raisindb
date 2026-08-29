// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Scheduled invocation callbacks: `raisin.scheduler.*`.
//!
//! One-shot, time-based function/flow runs from server-side functions:
//! `schedule` registers a delayed `ScheduledInvocation` job (restart-safe,
//! dispatched by the job system when `runAt` arrives), `cancel` revokes it
//! by job id or external key, and `list`/`get` inspect this repository's
//! invocations. All operations are scoped to the executing function's
//! tenant + repository.

use std::collections::HashMap;
use std::sync::Arc;

use raisin_error::Error;
use raisin_models::auth::AuthContext;
use raisin_rocksdb::{
    JobDataStore, META_ACTOR, META_EXTERNAL_KEY, META_INPUT, META_SCHEDULED_FOR, META_TARGET_PATH,
};
use raisin_storage::jobs::{JobContext, JobId, JobInfo, JobRegistry, JobStatus, JobType};
use serde_json::{json, Value};

use crate::api::{
    SchedulerCancelCallback, SchedulerGetCallback, SchedulerListCallback, SchedulerScheduleCallback,
};

const DEFAULT_BRANCH: &str = "main";
const FUNCTIONS_WORKSPACE: &str = "functions";

fn job_status_to_string(status: &JobStatus) -> &'static str {
    match status {
        JobStatus::Scheduled => "scheduled",
        JobStatus::Running | JobStatus::Executing => "running",
        JobStatus::Completed => "completed",
        JobStatus::Cancelled => "cancelled",
        JobStatus::Failed(_) => "failed",
    }
}

fn invocation_json(info: &JobInfo, context: &JobContext) -> Value {
    let (invocation_id, target_kind) = match &info.job_type {
        JobType::ScheduledInvocation {
            invocation_id,
            target_kind,
        } => (invocation_id.clone(), target_kind.clone()),
        _ => (String::new(), String::new()),
    };
    json!({
        "job_id": info.id.to_string(),
        "invocation_id": invocation_id,
        "target_kind": target_kind,
        "target_path": context.metadata.get(META_TARGET_PATH),
        "input": context.metadata.get(META_INPUT),
        "actor": context.metadata.get(META_ACTOR),
        "external_key": context.metadata.get(META_EXTERNAL_KEY),
        "run_at": context.metadata.get(META_SCHEDULED_FOR),
        "status": job_status_to_string(&info.status),
        "error": info.error,
        "result": info.result,
    })
}

/// Collect this tenant + repository's scheduled invocations.
async fn list_repo_invocations(
    registry: &JobRegistry,
    data_store: &JobDataStore,
    tenant_id: &str,
    repo_id: &str,
) -> Vec<(JobInfo, JobContext)> {
    let jobs = registry.list_jobs_by_tenant(tenant_id).await;
    let mut out = Vec::new();
    for job in jobs {
        if !matches!(job.job_type, JobType::ScheduledInvocation { .. }) {
            continue;
        }
        let Ok(Some(context)) = data_store.get(tenant_id, &job.id) else {
            continue;
        };
        if context.repo_id != repo_id {
            continue;
        }
        out.push((job, context));
    }
    out
}

/// The PENDING invocation carrying this external key, if there is one.
///
/// ── WHY ONLY PENDING ────────────────────────────────────────────────────────
/// This is what makes `externalKey` a uniqueness constraint rather than a
/// label, and the honest limit of that constraint is worth stating where it is
/// enforced: the registry keeps TERMINAL jobs for an hour
/// (`IN_MEMORY_RETENTION_HOURS`), so a completed invocation is simply gone from
/// this listing afterwards. A key therefore means "at most one invocation
/// WAITING TO RUN", never "this key has run at most once, ever" — that second
/// claim needs a durable record of its own, and a caller who needs it must keep
/// one (an inventory claim, a marker on the subject) rather than reading a
/// guarantee here that this cannot make.
async fn pending_invocation_with_key(
    registry: &JobRegistry,
    data_store: &JobDataStore,
    tenant_id: &str,
    repo_id: &str,
    external_key: &str,
) -> Option<(JobInfo, JobContext)> {
    list_repo_invocations(registry, data_store, tenant_id, repo_id)
        .await
        .into_iter()
        .find(|(info, ctx)| {
            matches!(info.status, JobStatus::Scheduled)
                && ctx.metadata.get(META_EXTERNAL_KEY).and_then(|v| v.as_str())
                    == Some(external_key)
        })
}

/// Resolve an invocation by job id first, falling back to external key.
async fn resolve_invocation(
    registry: &JobRegistry,
    data_store: &JobDataStore,
    tenant_id: &str,
    repo_id: &str,
    job_id_or_key: &str,
) -> Result<(JobInfo, JobContext), Error> {
    // Try as a job id first.
    let id = JobId::from_string(job_id_or_key.to_string());
    if let Ok(info) = registry.get_job_info(&id).await {
        if matches!(info.job_type, JobType::ScheduledInvocation { .. }) {
            if let Ok(Some(context)) = data_store.get(tenant_id, &id) {
                if context.repo_id == repo_id {
                    return Ok((info, context));
                }
            }
        }
    }

    // Fall back to the external key.
    list_repo_invocations(registry, data_store, tenant_id, repo_id)
        .await
        .into_iter()
        .find(|(_, ctx)| {
            ctx.metadata.get(META_EXTERNAL_KEY).and_then(|v| v.as_str()) == Some(job_id_or_key)
        })
        .ok_or_else(|| {
            Error::NotFound(format!(
                "No scheduled invocation with id or external key '{}'",
                job_id_or_key
            ))
        })
}

/// Create the scheduler_schedule callback: `raisin.scheduler.schedule(request)`.
pub fn create_scheduler_schedule(
    job_registry: Arc<JobRegistry>,
    job_data_store: Arc<JobDataStore>,
    tenant_id: String,
    repo_id: String,
    auth_context: Option<AuthContext>,
) -> SchedulerScheduleCallback {
    Arc::new(move |request: Value| {
        let job_registry = job_registry.clone();
        let job_data_store = job_data_store.clone();
        let tenant_id = tenant_id.clone();
        let repo_id = repo_id.clone();
        let actor = auth_context
            .as_ref()
            .and_then(|a| a.user_id.clone())
            .unwrap_or_else(|| "function".to_string());

        Box::pin(async move {
            let target_kind = request
                .get("targetKind")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Validation("schedule: 'targetKind' is required".to_string()))?
                .to_string();
            if target_kind != "function" && target_kind != "flow" {
                return Err(Error::Validation(format!(
                    "schedule: invalid targetKind '{}' (expected 'function' or 'flow')",
                    target_kind
                )));
            }
            let target_path = request
                .get("targetPath")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Validation("schedule: 'targetPath' is required".to_string()))?
                .to_string();
            let run_at_str = request
                .get("runAt")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Validation("schedule: 'runAt' is required".to_string()))?;
            let run_at = chrono::DateTime::parse_from_rfc3339(run_at_str)
                .map_err(|e| {
                    Error::Validation(format!(
                        "schedule: invalid 'runAt' timestamp '{}': {} (expected RFC3339)",
                        run_at_str, e
                    ))
                })?
                .with_timezone(&chrono::Utc);
            let input = request.get("input").cloned().unwrap_or_else(|| json!({}));
            let external_key = request
                .get("externalKey")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let branch = request
                .get("branch")
                .and_then(|v| v.as_str())
                .unwrap_or(DEFAULT_BRANCH)
                .to_string();
            let workspace = request
                .get("workspace")
                .and_then(|v| v.as_str())
                .unwrap_or(FUNCTIONS_WORKSPACE)
                .to_string();
            let max_retries = request
                .get("maxRetries")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32)
                .unwrap_or(0);

            // ── AN EXTERNAL KEY IS A UNIQUENESS CONSTRAINT ──────────────────
            // It used to be a label: the key went into the job's metadata, and
            // two schedules with the same key produced TWO jobs that both
            // fired. Every caller needing "arm this once" therefore had to
            // list-then-schedule, which is a check-then-write over an
            // eventually-consistent store — the exact race a scheduler exists
            // to remove. Enforcing it here is the only place it can be done
            // once for everyone.
            //
            // `replace: true` re-arms deliberately: the pending job is
            // cancelled and a new one registered, which is what a caller wants
            // when the moment itself has MOVED (an event rescheduled). Without
            // it a repeat schedule is a no-op that reports the job already
            // standing, so a materializer can run every minute and arm once.
            let replace = request
                .get("replace")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if let Some(key) = &external_key {
                if let Some((existing, context)) = pending_invocation_with_key(
                    &job_registry,
                    &job_data_store,
                    &tenant_id,
                    &repo_id,
                    key,
                )
                .await
                {
                    if replace {
                        job_registry.cancel_job(&existing.id).await?;
                    } else {
                        let mut out = invocation_json(&existing, &context);
                        // The caller asked for a schedule and got the one that
                        // already stands. Saying so beats reporting a fresh
                        // registration that did not happen.
                        out["deduplicated"] = json!(true);
                        return Ok(out);
                    }
                }
            }

            let invocation_id = nanoid::nanoid!();
            let job_type = JobType::ScheduledInvocation {
                invocation_id: invocation_id.clone(),
                target_kind,
            };

            let mut metadata = HashMap::new();
            metadata.insert(META_TARGET_PATH.to_string(), json!(target_path));
            metadata.insert(META_INPUT.to_string(), input);
            metadata.insert(META_ACTOR.to_string(), json!(actor));
            if let Some(key) = &external_key {
                metadata.insert(META_EXTERNAL_KEY.to_string(), json!(key));
            }
            metadata.insert(META_SCHEDULED_FOR.to_string(), json!(run_at.to_rfc3339()));

            let context = JobContext {
                tenant_id: tenant_id.clone(),
                repo_id,
                branch,
                workspace_id: workspace,
                revision: raisin_hlc::HLC::new(0, 0),
                metadata,
            };

            // Store job context BEFORE registering so dispatch can never
            // observe the job without its context.
            let job_id = JobId::new();
            job_data_store.put(&job_id, &context)?;

            // max_retries defaults to 0: a failed one-shot must not silently
            // re-fire unless the caller opts into retries.
            job_registry
                .register_job_at_with_id(
                    job_id.clone(),
                    job_type,
                    tenant_id,
                    run_at,
                    Some(max_retries),
                )
                .await?;

            Ok(json!({
                "job_id": job_id.to_string(),
                "invocation_id": invocation_id,
                "status": "scheduled",
                "run_at": run_at.to_rfc3339(),
            }))
        })
    })
}

/// Create the scheduler_cancel callback: `raisin.scheduler.cancel(jobIdOrKey)`.
pub fn create_scheduler_cancel(
    job_registry: Arc<JobRegistry>,
    job_data_store: Arc<JobDataStore>,
    tenant_id: String,
    repo_id: String,
) -> SchedulerCancelCallback {
    Arc::new(move |job_id_or_key: String| {
        let job_registry = job_registry.clone();
        let job_data_store = job_data_store.clone();
        let tenant_id = tenant_id.clone();
        let repo_id = repo_id.clone();

        Box::pin(async move {
            let (info, _context) = resolve_invocation(
                &job_registry,
                &job_data_store,
                &tenant_id,
                &repo_id,
                &job_id_or_key,
            )
            .await?;

            job_registry.cancel_job(&info.id).await?;

            Ok(json!({
                "job_id": info.id.to_string(),
                "status": "cancelled",
            }))
        })
    })
}

/// Create the scheduler_list callback: `raisin.scheduler.list(filter?)`.
pub fn create_scheduler_list(
    job_registry: Arc<JobRegistry>,
    job_data_store: Arc<JobDataStore>,
    tenant_id: String,
    repo_id: String,
) -> SchedulerListCallback {
    Arc::new(move |filter: Value| {
        let job_registry = job_registry.clone();
        let job_data_store = job_data_store.clone();
        let tenant_id = tenant_id.clone();
        let repo_id = repo_id.clone();

        Box::pin(async move {
            let external_key = filter
                .get("externalKey")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let status = filter
                .get("status")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            let invocations =
                list_repo_invocations(&job_registry, &job_data_store, &tenant_id, &repo_id).await;
            let items: Vec<Value> = invocations
                .iter()
                .filter(|(info, ctx)| {
                    if let Some(key) = &external_key {
                        if ctx.metadata.get(META_EXTERNAL_KEY).and_then(|v| v.as_str())
                            != Some(key.as_str())
                        {
                            return false;
                        }
                    }
                    if let Some(status) = &status {
                        if job_status_to_string(&info.status) != status {
                            return false;
                        }
                    }
                    true
                })
                .map(|(info, ctx)| invocation_json(info, ctx))
                .collect();

            Ok(json!({ "invocations": items }))
        })
    })
}

/// Create the scheduler_get callback: `raisin.scheduler.get(jobIdOrKey)`.
pub fn create_scheduler_get(
    job_registry: Arc<JobRegistry>,
    job_data_store: Arc<JobDataStore>,
    tenant_id: String,
    repo_id: String,
) -> SchedulerGetCallback {
    Arc::new(move |job_id_or_key: String| {
        let job_registry = job_registry.clone();
        let job_data_store = job_data_store.clone();
        let tenant_id = tenant_id.clone();
        let repo_id = repo_id.clone();

        Box::pin(async move {
            let (info, context) = resolve_invocation(
                &job_registry,
                &job_data_store,
                &tenant_id,
                &repo_id,
                &job_id_or_key,
            )
            .await?;

            Ok(invocation_json(&info, &context))
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_setup() -> (Arc<JobRegistry>, Arc<JobDataStore>, tempfile::TempDir) {
        let temp_dir = tempfile::tempdir().unwrap();
        let db = Arc::new(raisin_rocksdb::open_db(temp_dir.path()).unwrap());
        let data_store = Arc::new(JobDataStore::new(db));
        let registry = Arc::new(JobRegistry::new());
        (registry, data_store, temp_dir)
    }

    fn callbacks(
        registry: &Arc<JobRegistry>,
        data_store: &Arc<JobDataStore>,
        repo: &str,
    ) -> (
        SchedulerScheduleCallback,
        SchedulerCancelCallback,
        SchedulerListCallback,
        SchedulerGetCallback,
    ) {
        (
            create_scheduler_schedule(
                registry.clone(),
                data_store.clone(),
                "default".to_string(),
                repo.to_string(),
                None,
            ),
            create_scheduler_cancel(
                registry.clone(),
                data_store.clone(),
                "default".to_string(),
                repo.to_string(),
            ),
            create_scheduler_list(
                registry.clone(),
                data_store.clone(),
                "default".to_string(),
                repo.to_string(),
            ),
            create_scheduler_get(
                registry.clone(),
                data_store.clone(),
                "default".to_string(),
                repo.to_string(),
            ),
        )
    }

    fn schedule_request(external_key: Option<&str>) -> Value {
        let run_at = (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
        let mut req = json!({
            "targetKind": "function",
            "targetPath": "/lib/reminder",
            "input": {"note": "hi"},
            "runAt": run_at,
        });
        if let Some(key) = external_key {
            req["externalKey"] = json!(key);
        }
        req
    }

    #[tokio::test]
    async fn schedule_then_get_and_list_by_external_key() {
        let (registry, data_store, _tmp) = test_setup();
        let (schedule, _cancel, list, get) = callbacks(&registry, &data_store, "repo-a");

        let created = schedule(schedule_request(Some("order-42"))).await.unwrap();
        assert_eq!(created["status"], "scheduled");
        let job_id = created["job_id"].as_str().unwrap().to_string();

        // get by job id and by external key resolve the same invocation.
        let by_id = get(job_id.clone()).await.unwrap();
        assert_eq!(by_id["external_key"], json!("order-42"));
        assert_eq!(by_id["target_path"], json!("/lib/reminder"));
        assert_eq!(by_id["input"], json!({"note": "hi"}));
        assert_eq!(by_id["status"], "scheduled");
        let by_key = get("order-42".to_string()).await.unwrap();
        assert_eq!(by_key["job_id"], json!(job_id));

        // list filters by external key.
        let all = list(json!({})).await.unwrap();
        assert_eq!(all["invocations"].as_array().unwrap().len(), 1);
        let filtered = list(json!({"externalKey": "no-such-key"})).await.unwrap();
        assert!(filtered["invocations"].as_array().unwrap().is_empty());
    }

    // ── WHAT AN EXTERNAL KEY PROMISES ───────────────────────────────────────
    // These three hold the constraint to exactly what it claims. The middle one
    // is the reason it exists: without it a materializer that runs every minute
    // registers a fresh job every minute, and a reminder goes out as many times
    // as the sweep happened to tick before its moment arrived.

    #[tokio::test]
    async fn a_repeated_schedule_under_one_key_arms_a_single_job() {
        let (registry, data_store, _tmp) = test_setup();
        let (schedule, _cancel, list, _get) = callbacks(&registry, &data_store, "repo-a");

        let first = schedule(schedule_request(Some("invoice-9"))).await.unwrap();
        assert_eq!(first["status"], "scheduled");
        assert!(first.get("deduplicated").is_none());

        // The same caller, again — the ordinary case for anything that arms on
        // a schedule of its own rather than knowing it has already armed.
        let second = schedule(schedule_request(Some("invoice-9"))).await.unwrap();
        assert_eq!(
            second["deduplicated"],
            json!(true),
            "a repeat under a live key must report the job that already stands",
        );
        assert_eq!(
            second["job_id"], first["job_id"],
            "and it must be the SAME job, not a second one wearing the same key",
        );

        assert_eq!(
            list(json!({"externalKey": "invoice-9"}))
                .await
                .unwrap()["invocations"]
                .as_array()
                .unwrap()
                .len(),
            1,
            "one key, one pending invocation",
        );
    }

    #[tokio::test]
    async fn replace_rearms_the_key_at_the_new_moment() {
        let (registry, data_store, _tmp) = test_setup();
        let (schedule, _cancel, list, _get) = callbacks(&registry, &data_store, "repo-a");

        let first = schedule(schedule_request(Some("show-3"))).await.unwrap();

        // The moment MOVED — the show was rescheduled — so the standing job is
        // wrong and the caller says so explicitly.
        let mut moved = schedule_request(Some("show-3"));
        moved["runAt"] = json!((chrono::Utc::now() + chrono::Duration::hours(9)).to_rfc3339());
        moved["replace"] = json!(true);
        let second = schedule(moved).await.unwrap();

        assert_eq!(second["status"], "scheduled");
        assert_ne!(
            second["job_id"], first["job_id"],
            "replace registers a new job rather than editing the old one",
        );
        let pending = list(json!({"externalKey": "show-3"})).await.unwrap();
        let pending = pending["invocations"].as_array().unwrap();
        // The cancelled original may still be listed as history; exactly one is
        // still waiting to run, and that is the invariant that matters.
        assert_eq!(
            pending
                .iter()
                .filter(|i| i["status"] == "scheduled")
                .count(),
            1,
            "replacing must leave one live job, never two",
        );
    }

    #[tokio::test]
    async fn keyless_schedules_stay_independent() {
        let (registry, data_store, _tmp) = test_setup();
        let (schedule, _cancel, list, _get) = callbacks(&registry, &data_store, "repo-a");

        schedule(schedule_request(None)).await.unwrap();
        schedule(schedule_request(None)).await.unwrap();

        assert_eq!(
            list(json!({})).await.unwrap()["invocations"]
                .as_array()
                .unwrap()
                .len(),
            2,
            "deduplication is what a KEY buys; without one, two schedules are two jobs",
        );
    }

    #[tokio::test]
    async fn cancel_by_external_key_marks_cancelled() {
        let (registry, data_store, _tmp) = test_setup();
        let (schedule, cancel, list, get) = callbacks(&registry, &data_store, "repo-a");

        let created = schedule(schedule_request(Some("order-7"))).await.unwrap();
        let job_id = created["job_id"].as_str().unwrap().to_string();

        let cancelled = cancel("order-7".to_string()).await.unwrap();
        assert_eq!(cancelled["job_id"], json!(job_id));
        assert_eq!(cancelled["status"], "cancelled");

        let info = get(job_id).await.unwrap();
        assert_eq!(info["status"], "cancelled");
        let pending = list(json!({"status": "scheduled"})).await.unwrap();
        assert!(pending["invocations"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn operations_are_repo_scoped() {
        let (registry, data_store, _tmp) = test_setup();
        let (schedule_a, _cancel_a, _list_a, _get_a) = callbacks(&registry, &data_store, "repo-a");
        let (_schedule_b, cancel_b, list_b, get_b) = callbacks(&registry, &data_store, "repo-b");

        let created = schedule_a(schedule_request(Some("shared-key")))
            .await
            .unwrap();
        let job_id = created["job_id"].as_str().unwrap().to_string();

        // Another repository sees nothing and cannot cancel across repos.
        let listed = list_b(json!({})).await.unwrap();
        assert!(listed["invocations"].as_array().unwrap().is_empty());
        assert!(get_b(job_id.clone()).await.is_err());
        assert!(cancel_b("shared-key".to_string()).await.is_err());
        assert!(cancel_b(job_id).await.is_err());
    }

    #[tokio::test]
    async fn schedule_validates_input() {
        let (registry, data_store, _tmp) = test_setup();
        let (schedule, _cancel, _list, _get) = callbacks(&registry, &data_store, "repo-a");

        // Missing runAt
        let err = schedule(json!({"targetKind": "function", "targetPath": "/lib/x"}))
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Validation(_)));

        // Bad target kind
        let err = schedule(json!({
            "targetKind": "cron",
            "targetPath": "/lib/x",
            "runAt": chrono::Utc::now().to_rfc3339(),
        }))
        .await
        .unwrap_err();
        assert!(matches!(err, Error::Validation(_)));

        // Non-RFC3339 runAt
        let err = schedule(json!({
            "targetKind": "function",
            "targetPath": "/lib/x",
            "runAt": "tomorrow",
        }))
        .await
        .unwrap_err();
        assert!(matches!(err, Error::Validation(_)));
    }
}
