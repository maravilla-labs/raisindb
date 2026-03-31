use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use nanoid::nanoid;
use raisin_error::{Error as RaisinError, Result};
use raisin_hlc::HLC;
use raisin_models::nodes::types::archetype::Archetype;
use raisin_storage::scope::BranchScope;
use raisin_storage::{ArchetypeRepository, CommitMetadata};
use tokio::sync::RwLock;

#[derive(Clone)]
struct ArchetypeRevisionEntry {
    revision: HLC,
    archetype: Option<Archetype>,
}

impl ArchetypeRevisionEntry {
    fn new(revision: HLC, archetype: Option<Archetype>) -> Self {
        Self {
            revision,
            archetype,
        }
    }
}

#[derive(Default, Clone)]
pub struct InMemoryArchetypeRepo {
    revisions: Arc<RwLock<HashMap<String, Vec<ArchetypeRevisionEntry>>>>,
    id_to_name: Arc<RwLock<HashMap<String, String>>>,
    revision_counters: Arc<RwLock<HashMap<String, u64>>>,
    branch_heads: Arc<RwLock<HashMap<String, HLC>>>,
    version_index: Arc<RwLock<HashMap<String, HashMap<i32, HLC>>>>,
}

impl InMemoryArchetypeRepo {
    pub fn new() -> Self {
        Self::default()
    }

    fn repo_key(tenant_id: &str, repo_id: &str) -> String {
        format!("{tenant_id}/{repo_id}")
    }

    fn branch_key(tenant_id: &str, repo_id: &str, branch: &str) -> String {
        format!("{}/{}/{}", tenant_id, repo_id, branch)
    }

    fn archetype_key(tenant_id: &str, repo_id: &str, branch: &str, name: &str) -> String {
        format!("{}/{}/{}/{}", tenant_id, repo_id, branch, name)
    }

    fn branch_prefix(tenant_id: &str, repo_id: &str, branch: &str) -> String {
        format!("{}/{}/{}/", tenant_id, repo_id, branch)
    }

    fn id_key(tenant_id: &str, repo_id: &str, branch: &str, id: &str) -> String {
        format!("{}/{}/{}/id:{}", tenant_id, repo_id, branch, id)
    }

    async fn next_revision(&self, tenant_id: &str, repo_id: &str) -> HLC {
        let key = Self::repo_key(tenant_id, repo_id);
        let mut counters = self.revision_counters.write().await;
        let counter = counters.entry(key).or_insert(0);
        *counter += 1;
        HLC::new(*counter, 0)
    }

    async fn set_head(&self, tenant_id: &str, repo_id: &str, branch: &str, revision: HLC) {
        let key = Self::branch_key(tenant_id, repo_id, branch);
        let mut heads = self.branch_heads.write().await;
        heads.insert(key, revision);
    }

    async fn head(&self, tenant_id: &str, repo_id: &str, branch: &str) -> Option<HLC> {
        let key = Self::branch_key(tenant_id, repo_id, branch);
        let heads = self.branch_heads.read().await;
        heads.get(&key).copied()
    }

    fn resolve_at(entries: &[ArchetypeRevisionEntry], target_revision: &HLC) -> Option<Archetype> {
        for entry in entries.iter() {
            if &entry.revision <= target_revision {
                return entry.archetype.clone();
            }
        }
        None
    }
}

impl ArchetypeRepository for InMemoryArchetypeRepo {
    fn get(
        &self,
        scope: BranchScope<'_>,
        name: &str,
        max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<Option<Archetype>>> + Send {
        let tenant_id = scope.tenant_id;
        let repo_id = scope.repo_id;
        let branch_str = scope.branch;
        let key = Self::archetype_key(tenant_id, repo_id, branch_str, name);
        let branch = branch_str.to_string();
        let tenant = tenant_id.to_string();
        let repo = repo_id.to_string();
        let max_revision = max_revision.copied();
        async move {
            let target_revision = if let Some(max_rev) = max_revision {
                max_rev
            } else if let Some(head) = self.head(&tenant, &repo, &branch).await {
                head
            } else {
                return Ok(None);
            };

            let revisions = self.revisions.read().await;
            let entries = revisions.get(&key);
            let result = entries.and_then(|entries| Self::resolve_at(entries, &target_revision));
            Ok(result)
        }
    }

    fn get_by_id(
        &self,
        scope: BranchScope<'_>,
        id: &str,
        max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<Option<Archetype>>> + Send {
        let tenant_id = scope.tenant_id;
        let repo_id = scope.repo_id;
        let branch_str = scope.branch;
        let branch = branch_str.to_string();
        let tenant = tenant_id.to_string();
        let repo = repo_id.to_string();
        let id_key = Self::id_key(tenant_id, repo_id, branch_str, id);
        let max_revision = max_revision.copied();
        async move {
            let name_opt = {
                let id_map = self.id_to_name.read().await;
                id_map.get(&id_key).cloned()
            };

            if let Some(name) = name_opt {
                let scope = BranchScope::new(&tenant, &repo, &branch);
                self.get(scope, &name, max_revision.as_ref()).await
            } else {
                Ok(None)
            }
        }
    }

    fn get_by_names(
        &self,
        scope: BranchScope<'_>,
        names: &[String],
        max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<Vec<Archetype>>> + Send {
        let names = names.to_vec();
        let branch = scope.branch.to_string();
        let tenant = scope.tenant_id.to_string();
        let repo = scope.repo_id.to_string();
        let max_revision = max_revision.copied();
        async move {
            let mut results = Vec::new();
            for name in names {
                let scope = BranchScope::new(&tenant, &repo, &branch);
                if let Some(archetype) = self.get(scope, &name, max_revision.as_ref()).await? {
                    results.push(archetype);
                }
            }
            Ok(results)
        }
    }

    fn resolve_version_revision(
        &self,
        scope: BranchScope<'_>,
        name: &str,
        version: i32,
    ) -> impl std::future::Future<Output = Result<Option<HLC>>> + Send {
        let key = Self::archetype_key(scope.tenant_id, scope.repo_id, scope.branch, name);
        async move {
            let version_index = self.version_index.read().await;
            Ok(version_index
                .get(&key)
                .and_then(|versions| versions.get(&version))
                .copied())
        }
    }

    fn create(
        &self,
        scope: BranchScope<'_>,
        archetype: Archetype,
        commit: CommitMetadata,
    ) -> impl std::future::Future<Output = Result<HLC>> + Send {
        let branch = scope.branch.to_string();
        let tenant = scope.tenant_id.to_string();
        let repo = scope.repo_id.to_string();
        async move {
            let scope = BranchScope::new(&tenant, &repo, &branch);
            if self.get(scope, &archetype.name, None).await?.is_some() {
                return Err(RaisinError::AlreadyExists(format!(
                    "Archetype '{}' already exists",
                    archetype.name
                )));
            }

            let scope = BranchScope::new(&tenant, &repo, &branch);
            self.upsert(scope, archetype, commit).await
        }
    }

    fn update(
        &self,
        scope: BranchScope<'_>,
        archetype: Archetype,
        commit: CommitMetadata,
    ) -> impl std::future::Future<Output = Result<HLC>> + Send {
        let branch = scope.branch.to_string();
        let tenant = scope.tenant_id.to_string();
        let repo = scope.repo_id.to_string();
        async move {
            let scope = BranchScope::new(&tenant, &repo, &branch);
            if self.get(scope, &archetype.name, None).await?.is_none() {
                return Err(RaisinError::NotFound(format!(
                    "Archetype '{}' not found",
                    archetype.name
                )));
            }

            let scope = BranchScope::new(&tenant, &repo, &branch);
            self.upsert(scope, archetype, commit).await
        }
    }

    fn upsert(
        &self,
        scope: BranchScope<'_>,
        mut archetype: Archetype,
        commit: CommitMetadata,
    ) -> impl std::future::Future<Output = Result<HLC>> + Send {
        let branch = scope.branch.to_string();
        let tenant = scope.tenant_id.to_string();
        let repo = scope.repo_id.to_string();
        async move {
            let now = Utc::now();

            let scope = BranchScope::new(&tenant, &repo, &branch);
            let existing = self.get(scope, &archetype.name, None).await?;

            if archetype.id.is_empty() {
                archetype.id = nanoid!(16);
            }

            if let Some(existing_archetype) = existing {
                let next_version = existing_archetype.version.unwrap_or(0) + 1;
                archetype.version = Some(next_version);
                archetype.created_at = existing_archetype.created_at;
                archetype.previous_version = Some(existing_archetype.id);
            } else {
                archetype.version = Some(1);
                if archetype.created_at.is_none() {
                    archetype.created_at = Some(now);
                }
            }

            archetype.updated_at = Some(now);
            if archetype.publishable.unwrap_or(false) && archetype.published_by.is_none() {
                archetype.published_by = Some(commit.actor.clone());
            }

            let revision = self.next_revision(&tenant, &repo).await;
            let entry = ArchetypeRevisionEntry::new(revision, Some(archetype.clone()));
            let key = Self::archetype_key(&tenant, &repo, &branch, &archetype.name);

            {
                let mut revisions = self.revisions.write().await;
                revisions.entry(key).or_default().insert(0, entry);
            }

            if let Some(version) = archetype.version {
                let version_key = Self::archetype_key(&tenant, &repo, &branch, &archetype.name);
                let mut version_index = self.version_index.write().await;
                version_index
                    .entry(version_key)
                    .or_default()
                    .insert(version, revision);
            }

            {
                let mut id_map = self.id_to_name.write().await;
                let id_key = Self::id_key(&tenant, &repo, &branch, &archetype.id);
                id_map.insert(id_key, archetype.name.clone());
            }

            self.set_head(&tenant, &repo, &branch, revision).await;

            Ok(revision)
        }
    }

    fn delete(
        &self,
        scope: BranchScope<'_>,
        name: &str,
        commit: CommitMetadata,
    ) -> impl std::future::Future<Output = Result<Option<HLC>>> + Send {
        let branch = scope.branch.to_string();
        let tenant = scope.tenant_id.to_string();
        let repo = scope.repo_id.to_string();
        let name = name.to_string();
        let _commit = commit;
        async move {
            let scope = BranchScope::new(&tenant, &repo, &branch);
            let current = self.get(scope, &name, None).await?;

            let Some(existing) = current else {
                return Ok(None);
            };

            let revision = self.next_revision(&tenant, &repo).await;
            let key = Self::archetype_key(&tenant, &repo, &branch, &name);

            {
                let mut revisions = self.revisions.write().await;
                revisions
                    .entry(key)
                    .or_default()
                    .insert(0, ArchetypeRevisionEntry::new(revision, None));
            }

            {
                let mut id_map = self.id_to_name.write().await;
                let id_key = Self::id_key(&tenant, &repo, &branch, &existing.id);
                id_map.remove(&id_key);
            }

            self.set_head(&tenant, &repo, &branch, revision).await;

            Ok(Some(revision))
        }
    }

    fn list(
        &self,
        scope: BranchScope<'_>,
        max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<Vec<Archetype>>> + Send {
        let branch = scope.branch.to_string();
        let tenant = scope.tenant_id.to_string();
        let repo = scope.repo_id.to_string();
        let max_revision = max_revision.copied();
        async move {
            let target_revision = if let Some(max_rev) = max_revision {
                max_rev
            } else if let Some(head) = self.head(&tenant, &repo, &branch).await {
                head
            } else {
                return Ok(vec![]);
            };

            let prefix = Self::branch_prefix(&tenant, &repo, &branch);
            let revisions = self.revisions.read().await;
            let mut results = Vec::new();

            for (key, entries) in revisions.iter() {
                if key.starts_with(&prefix) {
                    if let Some(archetype) = Self::resolve_at(entries, &target_revision) {
                        results.push(archetype);
                    }
                }
            }

            results.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(results)
        }
    }

    fn list_published(
        &self,
        scope: BranchScope<'_>,
        max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<Vec<Archetype>>> + Send {
        let branch = scope.branch.to_string();
        let tenant = scope.tenant_id.to_string();
        let repo = scope.repo_id.to_string();
        let max_revision = max_revision.copied();
        async move {
            let scope = BranchScope::new(&tenant, &repo, &branch);
            let all = self.list(scope, max_revision.as_ref()).await?;

            Ok(all
                .into_iter()
                .filter(|archetype| archetype.publishable.unwrap_or(false))
                .collect())
        }
    }

    fn publish(
        &self,
        scope: BranchScope<'_>,
        name: &str,
        commit: CommitMetadata,
    ) -> impl std::future::Future<Output = Result<HLC>> + Send {
        let branch = scope.branch.to_string();
        let tenant = scope.tenant_id.to_string();
        let repo = scope.repo_id.to_string();
        let name = name.to_string();
        async move {
            let scope = BranchScope::new(&tenant, &repo, &branch);
            let mut archetype = self
                .get(scope, &name, None)
                .await?
                .ok_or_else(|| RaisinError::NotFound(format!("Archetype '{name}' not found")))?;

            let now = Utc::now();
            archetype.publishable = Some(true);
            archetype.published_at = Some(now);
            archetype.published_by = Some(commit.actor.clone());

            let scope = BranchScope::new(&tenant, &repo, &branch);
            self.upsert(scope, archetype, commit).await
        }
    }

    fn unpublish(
        &self,
        scope: BranchScope<'_>,
        name: &str,
        commit: CommitMetadata,
    ) -> impl std::future::Future<Output = Result<HLC>> + Send {
        let branch = scope.branch.to_string();
        let tenant = scope.tenant_id.to_string();
        let repo = scope.repo_id.to_string();
        let name = name.to_string();
        async move {
            let scope = BranchScope::new(&tenant, &repo, &branch);
            let mut archetype = self
                .get(scope, &name, None)
                .await?
                .ok_or_else(|| RaisinError::NotFound(format!("Archetype '{name}' not found")))?;

            archetype.publishable = Some(false);
            archetype.published_at = None;
            archetype.published_by = None;

            let scope = BranchScope::new(&tenant, &repo, &branch);
            self.upsert(scope, archetype, commit).await
        }
    }

    fn is_published(
        &self,
        scope: BranchScope<'_>,
        name: &str,
        max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<bool>> + Send {
        let branch = scope.branch.to_string();
        let tenant = scope.tenant_id.to_string();
        let repo = scope.repo_id.to_string();
        let name = name.to_string();
        let max_revision = max_revision.copied();
        async move {
            let scope = BranchScope::new(&tenant, &repo, &branch);
            let archetype = self.get(scope, &name, max_revision.as_ref()).await?;
            Ok(archetype
                .map(|arch| arch.publishable.unwrap_or(false))
                .unwrap_or(false))
        }
    }

    fn validate_published(
        &self,
        scope: BranchScope<'_>,
        archetype_name: &str,
        max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        let branch = scope.branch.to_string();
        let tenant = scope.tenant_id.to_string();
        let repo = scope.repo_id.to_string();
        let name = archetype_name.to_string();
        let max_revision = max_revision.copied();
        async move {
            let scope = BranchScope::new(&tenant, &repo, &branch);
            if self
                .is_published(scope, &name, max_revision.as_ref())
                .await?
            {
                Ok(())
            } else {
                Err(RaisinError::Validation(format!(
                    "Archetype '{}' is not published",
                    name
                )))
            }
        }
    }
}
