// SPDX-License-Identifier: BSL-1.1

//! Operator-triggered tenant maintenance: integrity scan, index verify and
//! rebuild, orphan cleanup, compaction, repair, and vector verify/rebuild/
//! regenerate.
//!
//! Same shape, and the same reason, as fulltext maintenance
//! (`fulltext/maintenance.rs`): the `/management/*/start` endpoints used to
//! register a job and then do the work in a detached task — or, for most of
//! them, do nothing at all behind a `TODO` — while the worker pool claimed the
//! contextless job and failed it. Here the worker that owns the job runs it,
//! stores the handler's return value as the job result and marks the terminal
//! status from that one outcome.
//!
//! Every operation is scoped to `context.tenant_id`. The transport derives that
//! tenant from the authenticated request and never from a request body, and the
//! storage scans underneath are bounded to the tenant's key prefix
//! (`crate::prefix_scan`).

use std::sync::{Arc, OnceLock};

use raisin_error::{Error, Result};
use raisin_storage::jobs::{JobContext, JobInfo, JobType};
use raisin_storage::{IndexType, Issue, ManagementOps};
use serde_json::{json, Value};

use crate::management::HnswManagement;

mod vector_regenerate;
use crate::RocksDBStorage;
pub use vector_regenerate::META_FORCE;

/// Metadata key: which RocksDB indexes an `IndexRebuild` job rebuilds
/// (`property` | `reference` | `child_order` | `all`; default `all`).
pub const META_INDEX_TYPE: &str = "index_type";

/// HNSW management is assembled after the job system starts (server startup),
/// so it is installed here once it exists. Vector jobs that run before that
/// fail with a clear error rather than pretending to succeed.
static VECTOR_MANAGEMENT: OnceLock<Arc<HnswManagement>> = OnceLock::new();

/// Make vector verify/rebuild jobs runnable. Called once at server startup.
pub fn install_vector_management(management: Arc<HnswManagement>) {
    let _ = VECTOR_MANAGEMENT.set(management);
}

pub struct MaintenanceJobHandler {
    storage: Arc<RocksDBStorage>,
}

impl MaintenanceJobHandler {
    pub fn new(storage: Arc<RocksDBStorage>) -> Self {
        Self { storage }
    }

    /// Whether this handler runs `job_type`.
    pub fn handles(job_type: &JobType) -> bool {
        matches!(
            job_type,
            JobType::IntegrityScan
                | JobType::IndexVerify
                | JobType::IndexRebuild
                | JobType::OrphanCleanup
                | JobType::Compaction
                | JobType::Repair
                | JobType::VectorVerify
                | JobType::VectorRebuild
                | JobType::VectorOptimize
                | JobType::VectorRegenerate
        )
    }

    pub async fn handle(&self, job: &JobInfo, context: &JobContext) -> Result<Option<Value>> {
        let tenant = context.tenant_id.as_str();
        let storage = self.storage.as_ref();
        // Only a global compaction (superadmin subtree) runs without a tenant;
        // everything else is a tenant operation and refuses to guess one.
        if tenant.is_empty() {
            return match &job.job_type {
                JobType::Compaction => Ok(Some(to_value(storage.compact(None).await?))),
                _ => Err(Error::Validation(
                    "maintenance job has no tenant in its context".to_string(),
                )),
            };
        }
        let result = match &job.job_type {
            JobType::IntegrityScan => to_value(storage.check_integrity(tenant).await?),
            JobType::IndexVerify => to_value(storage.verify_indexes(tenant).await?),
            JobType::IndexRebuild => {
                let index_type = index_type_of(context)?;
                to_value(storage.rebuild_indexes(tenant, index_type).await?)
            }
            JobType::OrphanCleanup => {
                json!({ "found": storage.cleanup_orphans(tenant).await?, "note": "orphaned nodes are reported, not deleted" })
            }
            JobType::Compaction => to_value(storage.compact(Some(tenant)).await?),
            JobType::Repair => self.repair(tenant).await?,
            // HNSW needs no optimisation pass; the job exists so the endpoint's
            // job-id contract holds. Say what happened.
            JobType::VectorOptimize => json!({
                "status": "no_op",
                "detail": "HNSW indexes need no optimisation"
            }),
            JobType::VectorRegenerate => {
                vector_regenerate::regenerate(storage, job, context).await?
            }
            JobType::VectorVerify | JobType::VectorRebuild => {
                let management = VECTOR_MANAGEMENT.get().ok_or_else(|| {
                    Error::storage(
                        "vector management is not initialised on this server".to_string(),
                    )
                })?;
                let (repo, branch) = (context.repo_id.as_str(), context.branch.as_str());
                if repo.is_empty() || branch.is_empty() {
                    return Err(Error::Validation(
                        "vector maintenance needs a repository and branch".to_string(),
                    ));
                }
                // A tenant without embeddings has no vector index to verify or
                // rebuild; that is an answer, not a failure.
                use raisin_embeddings::storage::TenantEmbeddingConfigStore;
                let configured = storage
                    .tenant_embedding_config_repository()
                    .get_config(tenant)
                    .ok()
                    .flatten()
                    .is_some();
                if !configured {
                    return Ok(Some(json!({
                        "status": "not_configured",
                        "detail": "embeddings are not configured for this tenant"
                    })));
                }
                if matches!(job.job_type, JobType::VectorVerify) {
                    to_value(management.verify_index(tenant, repo, branch).await?)
                } else {
                    to_value(
                        management
                            .rebuild_index(tenant, repo, branch, Some(job.id.clone()))
                            .await?,
                    )
                }
            }
            other => return Err(Error::Validation(format!("not a maintenance job: {other}"))),
        };
        Ok(Some(result))
    }

    /// Scan, fix what can be fixed with the tools that exist, scan again.
    ///
    /// The issue list is the server's own fresh scan, never a client-supplied
    /// list: a repair acts only on what this tenant's data actually shows.
    async fn repair(&self, tenant: &str) -> Result<Value> {
        let started = std::time::Instant::now();
        let storage = self.storage.as_ref();
        let before = storage.check_integrity(tenant).await?;

        let index_issues = before
            .issues_found
            .iter()
            .filter(|i| {
                matches!(
                    i,
                    Issue::MissingIndex { .. } | Issue::InconsistentIndex { .. }
                )
            })
            .count();
        let orphans = before
            .issues_found
            .iter()
            .filter(|i| matches!(i, Issue::OrphanedNode { .. }))
            .count();

        let mut actions = Vec::new();
        if index_issues > 0 {
            let stats = storage.rebuild_indexes(tenant, IndexType::All).await?;
            actions.push(
                json!({ "action": "rebuild_indexes", "for_issues": index_issues, "stats": stats }),
            );
        }
        if orphans > 0 {
            // `cleanup_orphans` reports orphans; it deletes nothing (deleting a
            // node on the word of one scan is not a repair). Say so.
            let found = storage.cleanup_orphans(tenant).await?;
            actions.push(json!({
                "action": "orphans_reported",
                "for_issues": orphans,
                "found": found,
                "note": "orphaned nodes are reported, not deleted"
            }));
        }

        let after = if actions.is_empty() {
            before.clone()
        } else {
            storage.check_integrity(tenant).await?
        };
        let count_by_kind = |issues: &[Issue]| {
            let mut m = std::collections::HashMap::<String, usize>::new();
            for i in issues {
                *m.entry(issue_kind(i).to_string()).or_default() += 1;
            }
            m
        };
        let (by_kind_before, by_kind_after) = (
            count_by_kind(&before.issues_found),
            count_by_kind(&after.issues_found),
        );
        // What went away, per kind. The same shape the admin console renders.
        let repairs_by_type: std::collections::HashMap<String, usize> = by_kind_before
            .iter()
            .filter_map(|(kind, n)| {
                let left = by_kind_after.get(kind).copied().unwrap_or(0);
                (n > &left).then(|| (kind.clone(), n - left))
            })
            .collect();
        // Kinds with no automatic repair (corrupted data, broken references,
        // duplicate children, missing workspaces) need an operator decision.
        let mut errors: Vec<String> = by_kind_after
            .iter()
            .map(|(kind, n)| format!("{kind}: {n} issue(s) not repaired automatically"))
            .collect();
        errors.sort();

        let result = raisin_storage::RepairResult {
            tenant: tenant.to_string(),
            issues_repaired: before
                .issues_found
                .len()
                .saturating_sub(after.issues_found.len()),
            issues_failed: after.issues_found.len(),
            repairs_by_type,
            duration_ms: started.elapsed().as_millis() as u64,
            errors,
        };
        let mut value = to_value(result);
        if let Value::Object(map) = &mut value {
            map.insert("actions".into(), json!(actions));
            map.insert("health_before".into(), json!(before.health_score));
            map.insert("health_after".into(), json!(after.health_score));
        }
        Ok(value)
    }
}

fn index_type_of(context: &JobContext) -> Result<IndexType> {
    match context
        .metadata
        .get(META_INDEX_TYPE)
        .and_then(|v| v.as_str())
        .unwrap_or("all")
    {
        "property" => Ok(IndexType::Property),
        "reference" => Ok(IndexType::Reference),
        "child_order" => Ok(IndexType::ChildOrder),
        "all" => Ok(IndexType::All),
        other => Err(Error::Validation(format!("invalid index type: {other}"))),
    }
}

fn issue_kind(issue: &Issue) -> &'static str {
    match issue {
        Issue::OrphanedNode { .. } => "orphaned_node",
        Issue::MissingIndex { .. } => "missing_index",
        Issue::InconsistentIndex { .. } => "inconsistent_index",
        Issue::CorruptedData { .. } => "corrupted_data",
        Issue::BrokenReference { .. } => "broken_reference",
        Issue::DuplicateChild { .. } => "duplicate_child",
        Issue::MissingWorkspace { .. } => "missing_workspace",
    }
}

fn to_value<T: serde::Serialize>(v: T) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}
