//! Repository management implementation

use crate::{cf, cf_handle, keys};
use raisin_context::{RepositoryConfig, RepositoryInfo};
use raisin_error::Result;
use raisin_events::EventBus;
use raisin_storage::RepositoryManagementRepository;
use rocksdb::DB;
use std::sync::Arc;

#[derive(Clone)]
pub struct RepositoryManagementRepositoryImpl {
    db: Arc<DB>,
    event_bus: Arc<dyn EventBus>,
    operation_capture: Option<Arc<crate::OperationCapture>>,
    /// The live job registry, so a deleted repository's queued and running
    /// jobs are cancelled before its keys go — otherwise a worker finishing
    /// one would write the repository straight back.
    job_registry: Option<Arc<raisin_storage::jobs::JobRegistry>>,
}

impl RepositoryManagementRepositoryImpl {
    pub fn new(db: Arc<DB>, event_bus: Arc<dyn EventBus>) -> Self {
        Self {
            db,
            event_bus,
            operation_capture: None,
            job_registry: None,
        }
    }

    /// Attach the job registry; see the field.
    pub fn with_job_registry(
        mut self,
        job_registry: Arc<raisin_storage::jobs::JobRegistry>,
    ) -> Self {
        self.job_registry = Some(job_registry);
        self
    }

    pub fn new_with_capture(
        db: Arc<DB>,
        event_bus: Arc<dyn EventBus>,
        operation_capture: Arc<crate::OperationCapture>,
    ) -> Self {
        Self {
            db,
            event_bus,
            operation_capture: Some(operation_capture),
            job_registry: None,
        }
    }
}

impl RepositoryManagementRepository for RepositoryManagementRepositoryImpl {
    async fn create_repository(
        &self,
        tenant_id: &str,
        repo_id: &str,
        config: RepositoryConfig,
    ) -> Result<RepositoryInfo> {
        let info = RepositoryInfo {
            tenant_id: tenant_id.to_string(),
            repo_id: repo_id.to_string(),
            created_at: chrono::Utc::now(),
            branches: Vec::new(),
            config: config.clone(),
        };

        let key = keys::repository_key(tenant_id, repo_id);
        let value = rmp_serde::to_vec(&info)
            .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

        let cf = cf_handle(&self.db, cf::REGISTRY)?;
        self.db
            .put_cf(cf, key, value)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        // Capture operation for replication
        if let Some(ref capture) = self.operation_capture {
            if capture.is_enabled() {
                let _op = capture
                    .capture_update_repository(
                        tenant_id.to_string(),
                        repo_id.to_string(),
                        info.clone(),
                        "system".to_string(),
                    )
                    .await;
                // Ignore capture errors - don't fail repository creation if replication fails
            }
        }

        // Emit RepositoryCreated event to trigger NodeType initialization
        let event = raisin_events::Event::Repository(raisin_events::RepositoryEvent {
            tenant_id: tenant_id.to_string(),
            repository_id: repo_id.to_string(),
            kind: raisin_events::RepositoryEventKind::Created,
            workspace: None,
            revision_id: None,
            branch_name: Some(config.default_branch.clone()),
            tag_name: None,
            message: None,
            actor: None,
            metadata: None,
        });

        self.event_bus.publish(event);

        Ok(info)
    }

    async fn get_repository(
        &self,
        tenant_id: &str,
        repo_id: &str,
    ) -> Result<Option<RepositoryInfo>> {
        let key = keys::repository_key(tenant_id, repo_id);
        let cf = cf_handle(&self.db, cf::REGISTRY)?;

        match self.db.get_cf(cf, key) {
            Ok(Some(bytes)) => {
                let info = rmp_serde::from_slice(&bytes).map_err(|e| {
                    raisin_error::Error::storage(format!("Deserialization error: {}", e))
                })?;
                Ok(Some(info))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(raisin_error::Error::storage(e.to_string())),
        }
    }

    async fn list_repositories(&self) -> Result<Vec<RepositoryInfo>> {
        // Repository keys are laid out as `{tenant}\0repos\0{repo}` (see
        // `keys::repository_key`), i.e. tenant-FIRST. There is no global
        // `repos\0...` prefix to scan, so a prefix iterator over "repos"
        // matches nothing and silently returned an empty list (which broke
        // e.g. the startup builtin-package scan over all tenants). Instead,
        // scan the whole REGISTRY column family and filter by key shape.
        let cf = cf_handle(&self.db, cf::REGISTRY)?;
        let iter = self.db.iterator_cf(cf, rocksdb::IteratorMode::Start);

        let mut repos = Vec::new();

        for item in iter {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            // Only keys of the exact shape `{tenant}\0repos\0{repo}` are
            // repository entries; the REGISTRY CF also holds other records
            // (e.g. `tenants\0{tenant_id}`).
            let parts: Vec<&[u8]> = key.split(|b| *b == 0).collect();
            if parts.len() != 3 || parts[1] != b"repos" {
                continue;
            }

            let info: RepositoryInfo = rmp_serde::from_slice(&value).map_err(|e| {
                raisin_error::Error::storage(format!("Deserialization error: {}", e))
            })?;
            repos.push(info);
        }

        Ok(repos)
    }

    async fn list_repositories_for_tenant(&self, tenant_id: &str) -> Result<Vec<RepositoryInfo>> {
        let prefix = keys::KeyBuilder::new()
            .push(tenant_id)
            .push("repos")
            .build_prefix();

        let cf = cf_handle(&self.db, cf::REGISTRY)?;
        let prefix_clone = prefix.clone();
        let iter = crate::prefix_scan(&self.db, cf, prefix);

        let mut repos = Vec::new();

        for item in iter {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            // Verify key actually starts with our prefix
            if !key.starts_with(&prefix_clone) {
                break;
            }
            let info: RepositoryInfo = rmp_serde::from_slice(&value).map_err(|e| {
                raisin_error::Error::storage(format!("Deserialization error: {}", e))
            })?;
            repos.push(info);
        }

        Ok(repos)
    }

    /// Delete a repository and EVERYTHING it owns. Irreversible.
    ///
    /// This used to remove the registry entry and nothing else: nodes,
    /// revisions, branches, translations, embeddings, indexes and jobs stayed,
    /// and a repository recreated under the same id came back with the old
    /// one's data. Now, in this order:
    ///
    /// 1. the repository's jobs are cancelled and dropped from the live
    ///    registry, so no worker writes into it while it is being removed;
    /// 2. every key it owns is removed from every column family
    ///    (`storage::repo_purge`, whose registry of column families is
    ///    exhaustive and test-enforced);
    /// 3. a `DeleteRepository` operation is captured, so cluster peers purge
    ///    their copy instead of replicating it back;
    /// 4. a `Deleted` repository event is published, so caches keyed by the
    ///    repository (SQL catalog, schema statistics) are dropped.
    ///
    /// Search index directories live outside RocksDB: the HTTP and WebSocket
    /// deletes remove them before answering, and the server's
    /// `RepositoryIndexPurgeHandler` removes them for a replicated delete.
    async fn delete_repository(&self, tenant_id: &str, repo_id: &str) -> Result<bool> {
        let key = keys::repository_key(tenant_id, repo_id);
        let cf = cf_handle(&self.db, cf::REGISTRY)?;

        let exists = self
            .db
            .get_cf(cf, &key)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?
            .is_some();
        if !exists {
            return Ok(false);
        }

        if let Some(registry) = &self.job_registry {
            cancel_repository_jobs(&self.db, registry, tenant_id, repo_id).await;
        }

        let report =
            crate::storage::repo_purge::purge_repository_keys(&self.db, tenant_id, repo_id);
        if report.failed.is_empty() {
            tracing::warn!(
                tenant_id,
                repo_id,
                column_families = report.cfs_purged,
                jobs = report.jobs_removed,
                "Repository deleted with all of its data"
            );
        } else {
            tracing::error!(
                tenant_id,
                repo_id,
                failed = ?report.failed,
                "Repository deleted, but some column families could not be purged"
            );
        }

        if let Some(ref capture) = self.operation_capture {
            if capture.is_enabled() {
                let _ = capture
                    .capture_delete_repository(
                        tenant_id.to_string(),
                        repo_id.to_string(),
                        "system".to_string(),
                    )
                    .await;
            }
        }

        self.event_bus.publish(raisin_events::Event::Repository(
            raisin_events::RepositoryEvent {
                tenant_id: tenant_id.to_string(),
                repository_id: repo_id.to_string(),
                kind: raisin_events::RepositoryEventKind::Deleted,
                workspace: None,
                revision_id: None,
                branch_name: None,
                tag_name: None,
                message: None,
                actor: None,
                metadata: None,
            },
        ));

        Ok(true)
    }

    async fn repository_exists(&self, tenant_id: &str, repo_id: &str) -> Result<bool> {
        Ok(self.get_repository(tenant_id, repo_id).await?.is_some())
    }

    async fn update_repository_config(
        &self,
        tenant_id: &str,
        repo_id: &str,
        config: RepositoryConfig,
    ) -> Result<()> {
        if let Some(mut info) = self.get_repository(tenant_id, repo_id).await? {
            let previous = std::mem::replace(&mut info.config, config);

            let key = keys::repository_key(tenant_id, repo_id);
            let value = rmp_serde::to_vec(&info)
                .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

            // A localized-name configuration change flips the index state in
            // the same batch (plan Phase 12).
            crate::localized_name::config_change::write_repository_record(
                &self.db,
                tenant_id,
                repo_id,
                &key,
                &value,
                Some(&previous),
                &info.config,
            )?;

            // Capture operation for replication
            if let Some(ref capture) = self.operation_capture {
                if capture.is_enabled() {
                    let _op = capture
                        .capture_update_repository(
                            tenant_id.to_string(),
                            repo_id.to_string(),
                            info.clone(),
                            "system".to_string(),
                        )
                        .await;
                    // Ignore capture errors - don't fail update if replication fails
                }
            }
        }

        Ok(())
    }
}

/// Cancel and forget every live job of the repository. Its persisted records
/// are removed by the key purge that follows.
async fn cancel_repository_jobs(
    db: &Arc<DB>,
    registry: &raisin_storage::jobs::JobRegistry,
    tenant_id: &str,
    repo_id: &str,
) {
    let data = crate::jobs::JobDataStore::new(db.clone());
    let mut ids = Vec::new();
    for job in registry.list_jobs_by_tenant(tenant_id).await {
        if let Ok(Some(context)) = data.get(tenant_id, &job.id) {
            if context.repo_id == repo_id {
                ids.push(job.id);
            }
        }
    }
    for id in &ids {
        let _ = registry.cancel_job(id).await;
        let _ = registry.delete_job(id).await;
    }
    if !ids.is_empty() {
        tracing::info!(
            tenant_id,
            repo_id,
            jobs = ids.len(),
            "Cancelled the deleted repository's jobs"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_repo() -> RepositoryManagementRepositoryImpl {
        let temp_dir = tempfile::tempdir().unwrap();
        let db = Arc::new(crate::open_db(temp_dir.path()).unwrap());
        // Keep tempdir alive for the test duration by leaking it (test-only).
        std::mem::forget(temp_dir);
        let event_bus = Arc::new(raisin_events::InMemoryEventBus::new());
        RepositoryManagementRepositoryImpl::new(db, event_bus)
    }

    /// Regression test: the global `list_repositories()` used to scan the
    /// prefix `repos\0`, but repository keys are `{tenant}\0repos\0{repo}`
    /// (tenant-first), so it always returned an empty list. That silently
    /// broke every all-tenant scan, most visibly the startup builtin-package
    /// auto-update which saw "0 repositories" and never reinstalled updated
    /// packages into existing repos.
    #[tokio::test]
    async fn test_list_repositories_returns_repos_across_tenants() {
        let repo_mgmt = make_repo();

        repo_mgmt
            .create_repository("tenant-a", "repo-1", RepositoryConfig::default())
            .await
            .unwrap();
        repo_mgmt
            .create_repository("tenant-a", "repo-2", RepositoryConfig::default())
            .await
            .unwrap();
        repo_mgmt
            .create_repository("tenant-b", "repo-3", RepositoryConfig::default())
            .await
            .unwrap();

        let all = repo_mgmt.list_repositories().await.unwrap();
        let mut pairs: Vec<(String, String)> = all
            .iter()
            .map(|r| (r.tenant_id.clone(), r.repo_id.clone()))
            .collect();
        pairs.sort();
        assert_eq!(
            pairs,
            vec![
                ("tenant-a".to_string(), "repo-1".to_string()),
                ("tenant-a".to_string(), "repo-2".to_string()),
                ("tenant-b".to_string(), "repo-3".to_string()),
            ],
            "global list_repositories must return every tenant's repositories"
        );

        // Per-tenant listing still scopes correctly.
        let tenant_a = repo_mgmt
            .list_repositories_for_tenant("tenant-a")
            .await
            .unwrap();
        assert_eq!(tenant_a.len(), 2);
        assert!(tenant_a.iter().all(|r| r.tenant_id == "tenant-a"));
    }
}
