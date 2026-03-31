//! NodeType CRUD operations (get, create, update, upsert, delete).

use super::NodeTypeRepositoryImpl;
use super::TOMBSTONE;
use crate::{cf, cf_handle, keys};
use raisin_error::{Error as RaisinError, Result};
use raisin_hlc::HLC;
use raisin_models::nodes::types::NodeType;
use raisin_models::tree::ChangeOperation;
use raisin_storage::scope::BranchScope;
use raisin_storage::{BranchRepository, CommitMetadata, NodeTypeRepository, RevisionRepository};
use rocksdb::WriteBatch;

impl NodeTypeRepository for NodeTypeRepositoryImpl {
    async fn get(
        &self,
        scope: BranchScope<'_>,
        name: &str,
        max_revision: Option<&HLC>,
    ) -> Result<Option<NodeType>> {
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
    ) -> Result<Option<NodeType>> {
        let all = self.list(scope, max_revision).await?;
        Ok(all.into_iter().find(|nt| nt.id.as_deref() == Some(id)))
    }

    async fn get_by_names(
        &self,
        scope: BranchScope<'_>,
        names: &[String],
        max_revision: Option<&HLC>,
    ) -> Result<Vec<NodeType>> {
        let mut result = Vec::new();
        for name in names {
            if let Some(nt) = self.get(scope, name, max_revision).await? {
                result.push(nt);
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
        let cf = cf_handle(&self.db, cf::NODE_TYPES)?;
        let key = keys::nodetype_version_index_key(tenant_id, repo_id, branch, name, version);
        match self.db.get_cf(cf, key) {
            Ok(Some(bytes)) => {
                if bytes.len() != 16 {
                    return Err(RaisinError::storage(
                        "Invalid nodetype version index entry (HLC length mismatch)",
                    ));
                }
                let hlc = HLC::decode_descending(&bytes).map_err(|e| {
                    RaisinError::storage(format!("Failed to decode nodetype version index: {}", e))
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
        node_type: NodeType,
        commit: CommitMetadata,
    ) -> Result<HLC> {
        // Check if NodeType already exists
        let existing = self.get(scope, &node_type.name, None).await?;

        if existing.is_some() {
            return Err(RaisinError::AlreadyExists(format!(
                "NodeType '{}' already exists",
                node_type.name
            )));
        }

        // Delegate to upsert for actual storage (we know it doesn't exist)
        self.upsert(scope, node_type, commit).await
    }

    async fn update(
        &self,
        scope: BranchScope<'_>,
        node_type: NodeType,
        commit: CommitMetadata,
    ) -> Result<HLC> {
        // Check if NodeType exists
        let existing = self.get(scope, &node_type.name, None).await?;

        if existing.is_none() {
            return Err(RaisinError::NotFound(format!(
                "NodeType '{}' not found",
                node_type.name
            )));
        }

        // Delegate to upsert for actual storage (we know it exists)
        self.upsert(scope, node_type, commit).await
    }

    async fn upsert(
        &self,
        scope: BranchScope<'_>,
        node_type: NodeType,
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

        let existing = self.get(scope, &node_type.name, None).await?;

        let enriched = Self::apply_versioning(node_type, existing.as_ref());
        let serialized = rmp_serde::to_vec_named(&enriched).map_err(|e| {
            RaisinError::storage(format!("Serialization error for NodeType: {}", e))
        })?;

        let revision = self.revision_repo.allocate_revision();

        let cf = cf_handle(&self.db, cf::NODE_TYPES)?;
        let key =
            keys::nodetype_key_versioned(tenant_id, repo_id, branch, &enriched.name, &revision);

        tracing::info!(
            target: "rocksb::nodetype::upsert",
            "Storing NodeType '{}' in {}/{}/{} at revision {}",
            enriched.name,
            tenant_id,
            repo_id,
            branch,
            revision
        );

        let mut batch = WriteBatch::default();
        batch.put_cf(cf, key, serialized);
        if let Some(version) = enriched.version {
            let index_key = keys::nodetype_version_index_key(
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
            .index_node_type_change(tenant_id, repo_id, &revision, &enriched.name)
            .await?;

        // Capture operation for replication
        if let Some(ref operation_capture) = self.operation_capture {
            if operation_capture.is_enabled() {
                let _ = operation_capture
                    .capture_upsert_nodetype(
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
        let parent_head = self
            .resolve_head_revision(tenant_id, repo_id, branch)
            .await?;

        let existing = self.get(scope, name, None).await?;

        if existing.is_none() {
            return Ok(None);
        }

        let revision = self.revision_repo.allocate_revision();

        let cf = cf_handle(&self.db, cf::NODE_TYPES)?;
        let key = keys::nodetype_key_versioned(tenant_id, repo_id, branch, name, &revision);

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
            .index_node_type_change(tenant_id, repo_id, &revision, name)
            .await?;

        // Capture operation for replication
        if let Some(ref operation_capture) = self.operation_capture {
            if operation_capture.is_enabled() {
                let _ = operation_capture
                    .capture_delete_nodetype(
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
    ) -> Result<Vec<NodeType>> {
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
            return Ok(Vec::new());
        };

        let cf = cf_handle(&self.db, cf::NODE_TYPES)?;
        let prefix = keys::nodetype_branch_prefix(tenant_id, repo_id, branch);
        let prefix_clone = prefix.clone();
        let iter = self.db.prefix_iterator_cf(cf, prefix);

        let mut seen = std::collections::HashSet::new();
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

            let nodetype = Self::deserialize_node_type(&value)?;
            result.push(nodetype);
        }

        result.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(result)
    }

    async fn list_published(
        &self,
        scope: BranchScope<'_>,
        max_revision: Option<&HLC>,
    ) -> Result<Vec<NodeType>> {
        let all = self.list(scope, max_revision).await?;
        Ok(all
            .into_iter()
            .filter(|nt| nt.published_at.is_some())
            .collect())
    }

    async fn publish(
        &self,
        scope: BranchScope<'_>,
        name: &str,
        commit: CommitMetadata,
    ) -> Result<HLC> {
        let mut node_type = self
            .get(scope, name, None)
            .await?
            .ok_or_else(|| RaisinError::NotFound(format!("NodeType '{}' not found", name)))?;

        node_type.published_at = Some(chrono::Utc::now());
        node_type.published_by = Some(commit.actor.clone());

        self.upsert(scope, node_type, commit).await
    }

    async fn unpublish(
        &self,
        scope: BranchScope<'_>,
        name: &str,
        commit: CommitMetadata,
    ) -> Result<HLC> {
        let mut node_type = self
            .get(scope, name, None)
            .await?
            .ok_or_else(|| RaisinError::NotFound(format!("NodeType '{}' not found", name)))?;

        node_type.published_at = None;
        node_type.published_by = None;

        self.upsert(scope, node_type, commit).await
    }

    async fn is_published(
        &self,
        scope: BranchScope<'_>,
        name: &str,
        max_revision: Option<&HLC>,
    ) -> Result<bool> {
        Ok(self
            .get(scope, name, max_revision)
            .await?
            .and_then(|nt| nt.published_at)
            .is_some())
    }

    async fn validate_published(
        &self,
        scope: BranchScope<'_>,
        node_type_name: &str,
        max_revision: Option<&HLC>,
    ) -> Result<()> {
        if !self
            .is_published(scope, node_type_name, max_revision)
            .await?
        {
            return Err(RaisinError::Validation(format!(
                "NodeType '{}' is not published",
                node_type_name
            )));
        }
        Ok(())
    }
}
