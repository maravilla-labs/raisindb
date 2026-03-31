use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use nanoid::nanoid;
use raisin_error::{Error as RaisinError, Result};
use raisin_hlc::HLC;
use raisin_models::nodes::types::element::element_type::ElementType;
use raisin_storage::scope::BranchScope;
use raisin_storage::{CommitMetadata, ElementTypeRepository};
use tokio::sync::RwLock;

#[derive(Clone)]
struct ElementTypeRevisionEntry {
    revision: HLC,
    element_type: Option<ElementType>,
}

impl ElementTypeRevisionEntry {
    fn new(revision: HLC, element_type: Option<ElementType>) -> Self {
        Self {
            revision,
            element_type,
        }
    }
}

#[derive(Default, Clone)]
pub struct InMemoryElementTypeRepo {
    revisions: Arc<RwLock<HashMap<String, Vec<ElementTypeRevisionEntry>>>>,
    id_to_name: Arc<RwLock<HashMap<String, String>>>,
    revision_counters: Arc<RwLock<HashMap<String, u64>>>,
    branch_heads: Arc<RwLock<HashMap<String, HLC>>>,
    version_index: Arc<RwLock<HashMap<String, HashMap<i32, HLC>>>>,
}

impl InMemoryElementTypeRepo {
    pub fn new() -> Self {
        Self::default()
    }

    fn repo_key(tenant_id: &str, repo_id: &str) -> String {
        format!("{tenant_id}/{repo_id}")
    }

    fn branch_key(tenant_id: &str, repo_id: &str, branch: &str) -> String {
        format!("{}/{}/{}", tenant_id, repo_id, branch)
    }

    fn element_type_key(tenant_id: &str, repo_id: &str, branch: &str, name: &str) -> String {
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

    fn resolve_at(
        entries: &[ElementTypeRevisionEntry],
        target_revision: &HLC,
    ) -> Option<ElementType> {
        for entry in entries.iter() {
            if &entry.revision <= target_revision {
                return entry.element_type.clone();
            }
        }
        None
    }
}

impl ElementTypeRepository for InMemoryElementTypeRepo {
    fn get(
        &self,
        scope: BranchScope<'_>,
        name: &str,
        max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<Option<ElementType>>> + Send {
        let tenant_id = scope.tenant_id;
        let repo_id = scope.repo_id;
        let branch = scope.branch;
        let key = Self::element_type_key(tenant_id, repo_id, branch, name);
        let branch = branch.to_string();
        let tenant = tenant_id.to_string();
        let repo = repo_id.to_string();
        async move {
            let max_revision = max_revision.copied();
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
    ) -> impl std::future::Future<Output = Result<Option<ElementType>>> + Send {
        let tenant_id = scope.tenant_id;
        let repo_id = scope.repo_id;
        let branch = scope.branch;
        let branch = branch.to_string();
        let tenant = tenant_id.to_string();
        let repo = repo_id.to_string();
        let id_key = Self::id_key(tenant_id, repo_id, branch.as_str(), id);
        let max_revision = max_revision.copied();
        async move {
            let name_opt = {
                let id_map = self.id_to_name.read().await;
                id_map.get(&id_key).cloned()
            };

            if let Some(name) = name_opt {
                self.get(
                    BranchScope::new(&tenant, &repo, &branch),
                    &name,
                    max_revision.as_ref(),
                )
                .await
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
    ) -> impl std::future::Future<Output = Result<Vec<ElementType>>> + Send {
        let names = names.to_vec();
        let tenant = scope.tenant_id.to_string();
        let repo = scope.repo_id.to_string();
        let branch = scope.branch.to_string();
        async move {
            let mut results = Vec::new();
            for name in names {
                if let Some(element_type) = self
                    .get(
                        BranchScope::new(&tenant, &repo, &branch),
                        &name,
                        max_revision,
                    )
                    .await?
                {
                    results.push(element_type);
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
        let key = Self::element_type_key(scope.tenant_id, scope.repo_id, scope.branch, name);
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
        element_type: ElementType,
        commit: CommitMetadata,
    ) -> impl std::future::Future<Output = Result<HLC>> + Send {
        let tenant = scope.tenant_id.to_string();
        let repo = scope.repo_id.to_string();
        let branch = scope.branch.to_string();
        async move {
            let bs = BranchScope::new(&tenant, &repo, &branch);
            if self.get(bs, &element_type.name, None).await?.is_some() {
                return Err(RaisinError::AlreadyExists(format!(
                    "ElementType '{}' already exists",
                    element_type.name
                )));
            }

            self.upsert(bs, element_type, commit).await
        }
    }

    fn update(
        &self,
        scope: BranchScope<'_>,
        element_type: ElementType,
        commit: CommitMetadata,
    ) -> impl std::future::Future<Output = Result<HLC>> + Send {
        let tenant = scope.tenant_id.to_string();
        let repo = scope.repo_id.to_string();
        let branch = scope.branch.to_string();
        async move {
            let bs = BranchScope::new(&tenant, &repo, &branch);
            if self.get(bs, &element_type.name, None).await?.is_none() {
                return Err(RaisinError::NotFound(format!(
                    "ElementType '{}' not found",
                    element_type.name
                )));
            }

            self.upsert(bs, element_type, commit).await
        }
    }

    fn upsert(
        &self,
        scope: BranchScope<'_>,
        mut element_type: ElementType,
        commit: CommitMetadata,
    ) -> impl std::future::Future<Output = Result<HLC>> + Send {
        let tenant = scope.tenant_id.to_string();
        let repo = scope.repo_id.to_string();
        let branch = scope.branch.to_string();
        async move {
            let now = Utc::now();
            let bs = BranchScope::new(&tenant, &repo, &branch);

            let existing = self.get(bs, &element_type.name, None).await?;

            if element_type.id.is_empty() {
                element_type.id = nanoid!(16);
            }

            if let Some(existing_element) = existing {
                let next_version = existing_element.version.unwrap_or(0) + 1;
                element_type.version = Some(next_version);
                element_type.created_at = existing_element.created_at;
                element_type.previous_version = Some(existing_element.id);
            } else {
                element_type.version = Some(1);
                if element_type.created_at.is_none() {
                    element_type.created_at = Some(now);
                }
            }

            element_type.updated_at = Some(now);
            if element_type.publishable.unwrap_or(false) && element_type.published_by.is_none() {
                element_type.published_by = Some(commit.actor.clone());
            }

            let revision = self.next_revision(&tenant, &repo).await;
            let entry = ElementTypeRevisionEntry::new(revision, Some(element_type.clone()));
            let key = Self::element_type_key(&tenant, &repo, &branch, &element_type.name);

            {
                let mut revisions = self.revisions.write().await;
                revisions.entry(key).or_default().insert(0, entry);
            }

            if let Some(version) = element_type.version {
                let version_key =
                    Self::element_type_key(&tenant, &repo, &branch, &element_type.name);
                let mut version_index = self.version_index.write().await;
                version_index
                    .entry(version_key)
                    .or_default()
                    .insert(version, revision);
            }

            {
                let mut id_map = self.id_to_name.write().await;
                let id_key = Self::id_key(&tenant, &repo, &branch, &element_type.id);
                id_map.insert(id_key, element_type.name.clone());
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
        let tenant = scope.tenant_id.to_string();
        let repo = scope.repo_id.to_string();
        let branch = scope.branch.to_string();
        let name = name.to_string();
        let _commit = commit;
        async move {
            let bs = BranchScope::new(&tenant, &repo, &branch);
            let current = self.get(bs, &name, None).await?;

            let Some(existing) = current else {
                return Ok(None);
            };

            let revision = self.next_revision(&tenant, &repo).await;
            let key = Self::element_type_key(&tenant, &repo, &branch, &name);

            {
                let mut revisions = self.revisions.write().await;
                revisions
                    .entry(key)
                    .or_default()
                    .insert(0, ElementTypeRevisionEntry::new(revision, None));
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
    ) -> impl std::future::Future<Output = Result<Vec<ElementType>>> + Send {
        let tenant = scope.tenant_id.to_string();
        let repo = scope.repo_id.to_string();
        let branch = scope.branch.to_string();
        async move {
            let max_revision = max_revision.copied();
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
                    if let Some(element_type) = Self::resolve_at(entries, &target_revision) {
                        results.push(element_type);
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
    ) -> impl std::future::Future<Output = Result<Vec<ElementType>>> + Send {
        let tenant = scope.tenant_id.to_string();
        let repo = scope.repo_id.to_string();
        let branch = scope.branch.to_string();
        let max_revision = max_revision.copied();
        async move {
            let all = self
                .list(
                    BranchScope::new(&tenant, &repo, &branch),
                    max_revision.as_ref(),
                )
                .await?;

            Ok(all
                .into_iter()
                .filter(|element_type| element_type.publishable.unwrap_or(false))
                .collect())
        }
    }

    fn publish(
        &self,
        scope: BranchScope<'_>,
        name: &str,
        commit: CommitMetadata,
    ) -> impl std::future::Future<Output = Result<HLC>> + Send {
        let tenant = scope.tenant_id.to_string();
        let repo = scope.repo_id.to_string();
        let branch = scope.branch.to_string();
        let name = name.to_string();
        async move {
            let bs = BranchScope::new(&tenant, &repo, &branch);
            let mut element_type = self
                .get(bs, &name, None)
                .await?
                .ok_or_else(|| RaisinError::NotFound(format!("ElementType '{name}' not found")))?;

            let now = Utc::now();
            element_type.publishable = Some(true);
            element_type.published_at = Some(now);
            element_type.published_by = Some(commit.actor.clone());

            self.upsert(bs, element_type, commit).await
        }
    }

    fn unpublish(
        &self,
        scope: BranchScope<'_>,
        name: &str,
        commit: CommitMetadata,
    ) -> impl std::future::Future<Output = Result<HLC>> + Send {
        let tenant = scope.tenant_id.to_string();
        let repo = scope.repo_id.to_string();
        let branch = scope.branch.to_string();
        let name = name.to_string();
        async move {
            let bs = BranchScope::new(&tenant, &repo, &branch);
            let mut element_type = self
                .get(bs, &name, None)
                .await?
                .ok_or_else(|| RaisinError::NotFound(format!("ElementType '{name}' not found")))?;

            element_type.publishable = Some(false);
            element_type.published_at = None;
            element_type.published_by = None;

            self.upsert(bs, element_type, commit).await
        }
    }

    fn is_published(
        &self,
        scope: BranchScope<'_>,
        name: &str,
        max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<bool>> + Send {
        let tenant = scope.tenant_id.to_string();
        let repo = scope.repo_id.to_string();
        let branch = scope.branch.to_string();
        let name = name.to_string();
        async move {
            let element_type = self
                .get(
                    BranchScope::new(&tenant, &repo, &branch),
                    &name,
                    max_revision,
                )
                .await?;
            Ok(element_type
                .map(|el| el.publishable.unwrap_or(false))
                .unwrap_or(false))
        }
    }

    fn validate_published(
        &self,
        scope: BranchScope<'_>,
        element_type_name: &str,
        max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        let tenant = scope.tenant_id.to_string();
        let repo = scope.repo_id.to_string();
        let branch = scope.branch.to_string();
        let name = element_type_name.to_string();
        async move {
            if self
                .is_published(
                    BranchScope::new(&tenant, &repo, &branch),
                    &name,
                    max_revision,
                )
                .await?
            {
                Ok(())
            } else {
                Err(RaisinError::Validation(format!(
                    "ElementType '{}' is not published",
                    name
                )))
            }
        }
    }
}
