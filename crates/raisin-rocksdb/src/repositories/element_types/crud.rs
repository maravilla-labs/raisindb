//! ElementType CRUD operations (get, create, update, upsert, delete, list, publish).

use super::ElementTypeRepositoryImpl;
use super::TOMBSTONE;
use crate::{cf, cf_handle, keys};
use raisin_error::{Error as RaisinError, Result};
use raisin_hlc::HLC;
use raisin_models::nodes::types::element::element_type::ElementType;
use raisin_models::tree::ChangeOperation;
use raisin_storage::scope::BranchScope;
use raisin_storage::{BranchRepository, CommitMetadata, ElementTypeRepository, RevisionRepository};
use rocksdb::WriteBatch;
use std::collections::HashSet;

impl ElementTypeRepository for ElementTypeRepositoryImpl {
    async fn get(
        &self,
        scope: BranchScope<'_>,
        name: &str,
        max_revision: Option<&HLC>,
    ) -> Result<Option<ElementType>> {
        let BranchScope {
            tenant_id,
            repo_id,
            branch,
        } = scope;
        let target_revision = if let Some(rev) = max_revision {
            *rev
        } else if let Some(head) = self
            .resolve_head_revision(tenant_id, repo_id, branch)
            .await?
        {
            head
        } else {
            return Ok(None);
        };

        self.get_at_or_before(tenant_id, repo_id, branch, name, &target_revision)
            .await
    }

    async fn get_by_id(
        &self,
        scope: BranchScope<'_>,
        id: &str,
        max_revision: Option<&HLC>,
    ) -> Result<Option<ElementType>> {
        let all = self.list(scope, max_revision).await?;
        Ok(all.into_iter().find(|el| el.id == id))
    }

    async fn get_by_names(
        &self,
        scope: BranchScope<'_>,
        names: &[String],
        max_revision: Option<&HLC>,
    ) -> Result<Vec<ElementType>> {
        let mut result = Vec::new();
        for name in names {
            if let Some(element_type) = self.get(scope, name, max_revision).await? {
                result.push(element_type);
            }
        }
        Ok(result)
    }

    async fn resolve_version_revision(
        &self,
        scope: BranchScope<'_>,
        name: &str,
        version: i32,
    ) -> Result<Option<HLC>> {
        let BranchScope {
            tenant_id,
            repo_id,
            branch,
        } = scope;
        let cf = cf_handle(&self.db, cf::ELEMENT_TYPES)?;
        let key = keys::element_type_version_index_key(tenant_id, repo_id, branch, name, version);
        match self.db.get_cf(cf, key) {
            Ok(Some(bytes)) => {
                if bytes.len() != 16 {
                    return Err(RaisinError::storage(
                        "Invalid element type version index entry (HLC length mismatch)",
                    ));
                }
                let hlc = HLC::decode_descending(&bytes).map_err(|e| {
                    RaisinError::storage(format!(
                        "Failed to decode element type version index: {}",
                        e
                    ))
                })?;
                Ok(Some(hlc))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(RaisinError::storage(e.to_string())),
        }
    }

    async fn create(
        &self,
        scope: BranchScope<'_>,
        element_type: ElementType,
        commit: CommitMetadata,
    ) -> Result<HLC> {
        let existing = self.get(scope, &element_type.name, None).await?;

        if existing.is_some() {
            return Err(RaisinError::AlreadyExists(format!(
                "ElementType '{}' already exists",
                element_type.name
            )));
        }

        self.upsert(scope, element_type, commit).await
    }

    async fn update(
        &self,
        scope: BranchScope<'_>,
        element_type: ElementType,
        commit: CommitMetadata,
    ) -> Result<HLC> {
        let existing = self.get(scope, &element_type.name, None).await?;

        if existing.is_none() {
            return Err(RaisinError::NotFound(format!(
                "ElementType '{}' not found",
                element_type.name
            )));
        }

        self.upsert(scope, element_type, commit).await
    }

    async fn upsert(
        &self,
        scope: BranchScope<'_>,
        element_type: ElementType,
        commit: CommitMetadata,
    ) -> Result<HLC> {
        let BranchScope {
            tenant_id,
            repo_id,
            branch,
        } = scope;
        let parent_head = self
            .resolve_head_revision(tenant_id, repo_id, branch)
            .await?;

        let existing = self.get(scope, &element_type.name, None).await?;

        let enriched = Self::apply_versioning(element_type, existing.as_ref());
        let serialized = rmp_serde::to_vec_named(&enriched).map_err(|e| {
            RaisinError::storage(format!("Serialization error for ElementType: {}", e))
        })?;

        let revision = self.revision_repo.allocate_revision();

        let cf = cf_handle(&self.db, cf::ELEMENT_TYPES)?;
        let key =
            keys::element_type_key_versioned(tenant_id, repo_id, branch, &enriched.name, &revision);

        let mut batch = WriteBatch::default();
        batch.put_cf(cf, key, serialized);
        if let Some(version) = enriched.version {
            let index_key = keys::element_type_version_index_key(
                tenant_id,
                repo_id,
                branch,
                &enriched.name,
                version,
            );
            batch.put_cf(cf, index_key, revision.encode_descending());
        }

        self.db
            .write(batch)
            .map_err(|e| RaisinError::storage(e.to_string()))?;

        self.branch_repo
            .update_head(tenant_id, repo_id, branch, revision)
            .await?;

        let operation = if existing.is_some() {
            ChangeOperation::Modified
        } else {
            ChangeOperation::Added
        };

        let revision_meta = Self::build_revision_meta(
            revision,
            Self::determine_parent(parent_head),
            branch,
            &commit,
            operation,
            &enriched.name,
        );

        self.revision_repo
            .store_revision_meta(tenant_id, repo_id, revision_meta)
            .await?;
        self.revision_repo
            .index_element_type_change(tenant_id, repo_id, &revision, &enriched.name)
            .await?;

        // Capture operation for replication
        if let Some(ref operation_capture) = self.operation_capture {
            if operation_capture.is_enabled() {
                let _ = operation_capture
                    .capture_upsert_element_type(
                        tenant_id.to_string(),
                        repo_id.to_string(),
                        branch.to_string(),
                        enriched.name.clone(),
                        enriched.clone(),
                        commit.actor.clone(),
                        revision,
                    )
                    .await;
            }
        }

        Ok(revision)
    }

    async fn delete(
        &self,
        scope: BranchScope<'_>,
        name: &str,
        commit: CommitMetadata,
    ) -> Result<Option<HLC>> {
        let BranchScope {
            tenant_id,
            repo_id,
            branch,
        } = scope;
        let existing = self.get(scope, name, None).await?;

        let Some(_existing) = existing else {
            return Ok(None);
        };

        let parent_head = self
            .resolve_head_revision(tenant_id, repo_id, branch)
            .await?;

        let revision = self.revision_repo.allocate_revision();

        let cf = cf_handle(&self.db, cf::ELEMENT_TYPES)?;
        let key = keys::element_type_key_versioned(tenant_id, repo_id, branch, name, &revision);

        let mut batch = WriteBatch::default();
        batch.put_cf(cf, key, TOMBSTONE);

        self.db
            .write(batch)
            .map_err(|e| RaisinError::storage(e.to_string()))?;

        self.branch_repo
            .update_head(tenant_id, repo_id, branch, revision)
            .await?;

        let revision_meta = Self::build_revision_meta(
            revision,
            Self::determine_parent(parent_head),
            branch,
            &commit,
            ChangeOperation::Deleted,
            name,
        );

        self.revision_repo
            .store_revision_meta(tenant_id, repo_id, revision_meta)
            .await?;
        self.revision_repo
            .index_element_type_change(tenant_id, repo_id, &revision, name)
            .await?;

        // Capture operation for replication
        if let Some(ref operation_capture) = self.operation_capture {
            if operation_capture.is_enabled() {
                let _ = operation_capture
                    .capture_delete_element_type(
                        tenant_id.to_string(),
                        repo_id.to_string(),
                        branch.to_string(),
                        name.to_string(),
                        commit.actor.clone(),
                        revision,
                    )
                    .await;
            }
        }

        Ok(Some(revision))
    }

    async fn list(
        &self,
        scope: BranchScope<'_>,
        max_revision: Option<&HLC>,
    ) -> Result<Vec<ElementType>> {
        let BranchScope {
            tenant_id,
            repo_id,
            branch,
        } = scope;
        let target_revision = if let Some(rev) = max_revision {
            *rev
        } else if let Some(head) = self
            .resolve_head_revision(tenant_id, repo_id, branch)
            .await?
        {
            head
        } else {
            return Ok(vec![]);
        };

        let cf = cf_handle(&self.db, cf::ELEMENT_TYPES)?;
        let prefix = keys::element_type_branch_prefix(tenant_id, repo_id, branch);
        let prefix_clone = prefix.clone();
        let iter = self.db.prefix_iterator_cf(cf, prefix.clone());

        let mut seen = HashSet::new();
        let mut result = Vec::new();

        for item in iter {
            let (key, value) = item.map_err(|e| RaisinError::storage(e.to_string()))?;
            if !key.starts_with(&prefix_clone) {
                break;
            }

            let name = Self::extract_name(&key)?;
            if seen.contains(&name) {
                continue;
            }

            let revision = Self::decode_revision(&key)?;
            if revision > target_revision {
                continue;
            }

            seen.insert(name.clone());

            if value.as_ref() == TOMBSTONE {
                continue;
            }

            let element_type = Self::deserialize(&value)?;
            result.push(element_type);
        }

        result.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(result)
    }

    async fn list_published(
        &self,
        scope: BranchScope<'_>,
        max_revision: Option<&HLC>,
    ) -> Result<Vec<ElementType>> {
        let all = self.list(scope, max_revision).await?;
        Ok(all
            .into_iter()
            .filter(|el| el.publishable.unwrap_or(false))
            .collect())
    }

    async fn publish(
        &self,
        scope: BranchScope<'_>,
        name: &str,
        commit: CommitMetadata,
    ) -> Result<HLC> {
        let mut element_type = self
            .get(scope, name, None)
            .await?
            .ok_or_else(|| RaisinError::NotFound(format!("ElementType '{}' not found", name)))?;

        element_type.publishable = Some(true);
        element_type.published_at = Some(chrono::Utc::now());
        element_type.published_by = Some(commit.actor.clone());

        self.upsert(scope, element_type, commit).await
    }

    async fn unpublish(
        &self,
        scope: BranchScope<'_>,
        name: &str,
        commit: CommitMetadata,
    ) -> Result<HLC> {
        let mut element_type = self
            .get(scope, name, None)
            .await?
            .ok_or_else(|| RaisinError::NotFound(format!("ElementType '{}' not found", name)))?;

        element_type.publishable = Some(false);
        element_type.published_at = None;
        element_type.published_by = None;

        self.upsert(scope, element_type, commit).await
    }

    async fn is_published(
        &self,
        scope: BranchScope<'_>,
        name: &str,
        max_revision: Option<&HLC>,
    ) -> Result<bool> {
        let element_type = self.get(scope, name, max_revision).await?;
        Ok(element_type
            .map(|el| el.publishable.unwrap_or(false))
            .unwrap_or(false))
    }

    async fn validate_published(
        &self,
        scope: BranchScope<'_>,
        element_type_name: &str,
        max_revision: Option<&HLC>,
    ) -> Result<()> {
        if self
            .is_published(scope, element_type_name, max_revision)
            .await?
        {
            Ok(())
        } else {
            Err(RaisinError::Validation(format!(
                "ElementType '{}' is not published",
                element_type_name
            )))
        }
    }
}
