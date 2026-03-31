//! Tag repository implementation

use crate::{cf, cf_handle, keys};
use raisin_context::Tag;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_storage::TagRepository;
use rocksdb::DB;
use std::sync::Arc;

#[derive(Clone)]
pub struct TagRepositoryImpl {
    db: Arc<DB>,
}

impl TagRepositoryImpl {
    pub fn new(db: Arc<DB>) -> Self {
        Self { db }
    }

    /// Physically copy all revision-aware indexes from source branch to tag name
    /// Keeps the same revisions, only changes the branch name in the key to the tag name
    async fn copy_tag_indexes(
        &self,
        tenant_id: &str,
        repo_id: &str,
        source_branch: &str,
        tag_name: &str,
        max_revision: HLC,
    ) -> Result<()> {
        // Copy indexes from these column families:
        // 1. NODES - actual node data
        // 2. PATH_INDEX - path-based lookups
        // 3. ORDERED_CHILDREN - child ordering
        // 4. NODE_TYPES - schema definitions and version indexes

        let cfs_to_copy = vec![
            (cf::NODES, "nodes"),
            (cf::PATH_INDEX, "path_index"),
            (cf::ORDERED_CHILDREN, "ordered_children"),
            (cf::NODE_TYPES, "node_types"),
            (cf::ARCHETYPES, "archetypes"),
            (cf::ELEMENT_TYPES, "element_types"),
            (cf::NODE_PATH, "node_path"),
        ];

        for (cf_name, display_name) in cfs_to_copy {
            let copied = self
                .copy_cf_entries(
                    tenant_id,
                    repo_id,
                    source_branch,
                    tag_name,
                    &max_revision,
                    cf_name,
                )
                .await?;

            tracing::info!(
                "Copied {} entries from {} CF (branch: {} -> tag: {})",
                copied,
                display_name,
                source_branch,
                tag_name
            );
        }

        Ok(())
    }

    /// Copy entries from a specific column family, preserving revisions
    async fn copy_cf_entries(
        &self,
        tenant_id: &str,
        repo_id: &str,
        source_branch: &str,
        tag_name: &str,
        max_revision: &HLC,
        cf_name: &str,
    ) -> Result<usize> {
        let cf = cf_handle(&self.db, cf_name)?;

        // Build prefix for source branch: {tenant}\0{repo}\0{source_branch}\0
        let source_prefix = keys::KeyBuilder::new()
            .push(tenant_id)
            .push(repo_id)
            .push(source_branch)
            .build_prefix();

        let source_prefix_clone = source_prefix.clone();

        // For ORDERED_CHILDREN, use iterator_cf with seek instead of prefix_iterator_cf
        // because the CF has a custom prefix extractor configured
        use rocksdb::IteratorMode;
        let iter = if cf_name == cf::ORDERED_CHILDREN {
            self.db.iterator_cf(
                &cf,
                IteratorMode::From(&source_prefix_clone, rocksdb::Direction::Forward),
            )
        } else {
            self.db.prefix_iterator_cf(&cf, source_prefix)
        };

        let mut copied_count = 0;

        for item in iter {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            // Verify key actually starts with our prefix
            if !key.starts_with(&source_prefix_clone) {
                break;
            }

            // Parse the key to extract the revision
            // Key formats:
            // NODES: {tenant}\0{repo}\0{branch}\0{workspace}\0nodes\0{node_id}\0{~revision}
            // PATH_INDEX: {tenant}\0{repo}\0{branch}\0{workspace}\0path\0{path}\0{~revision}
            // ORDERED_CHILDREN: {tenant}\0{repo}\0{branch}\0{workspace}\0ordered\0{parent_id}\0{order_label}\0{~revision}\0{child_id}

            let parts: Vec<&[u8]> = key.split(|&b| b == 0).collect();

            // Find the revision bytes (they're encoded as descending u64)
            let revision_opt = if cf_name == cf::NODE_TYPES {
                match parts.get(3).copied() {
                    Some(segment) if segment == b"nodetypes" => {
                        if let Some(rev_part) = parts.get(5) {
                            if rev_part.len() == 16 {
                                keys::decode_descending_revision(rev_part).ok()
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    }
                    Some(segment) if segment == b"nodetype_versions" => {
                        let value_slice = value.as_ref();
                        if value_slice.len() == 16 {
                            keys::decode_descending_revision(value_slice).ok()
                        } else {
                            None
                        }
                    }
                    _ => None,
                }
            } else if cf_name == cf::ORDERED_CHILDREN && parts.len() >= 9 {
                // ORDERED_CHILDREN: revision is at index 7
                if parts[7].len() == 16 {
                    keys::decode_descending_revision(parts[7]).ok()
                } else {
                    None
                }
            } else if parts.len() >= 7 {
                // NODES and PATH_INDEX: revision is at index 6
                // (0=tenant, 1=repo, 2=branch, 3=workspace, 4=type, 5=id/path, 6=revision)
                if parts[6].len() == 16 {
                    keys::decode_descending_revision(parts[6]).ok()
                } else {
                    None
                }
            } else {
                None
            };

            // Only copy if revision <= max_revision
            if let Some(revision) = revision_opt {
                if &revision <= max_revision {
                    // Create new key with tag name instead of source branch
                    // Replace the branch component (index 2) with tag_name
                    let mut new_key_parts = parts.iter().map(|p| p.to_vec()).collect::<Vec<_>>();
                    new_key_parts[2] = tag_name.as_bytes().to_vec();

                    // Rebuild the key
                    let mut new_key = Vec::new();
                    for (i, part) in new_key_parts.iter().enumerate() {
                        if i > 0 {
                            new_key.push(0); // null byte separator
                        }
                        new_key.extend_from_slice(part);
                    }

                    // Write to tag with same value (preserving revision)
                    self.db
                        .put_cf(&cf, new_key, &value)
                        .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

                    copied_count += 1;
                }
            }
        }

        Ok(copied_count)
    }
}

impl TagRepository for TagRepositoryImpl {
    async fn create_tag(
        &self,
        tenant_id: &str,
        repo_id: &str,
        tag_name: &str,
        revision: &HLC,
        created_by: &str,
        message: Option<String>,
        protected: bool,
    ) -> Result<Tag> {
        let tag = Tag {
            name: tag_name.to_string(),
            revision: *revision,
            created_at: chrono::Utc::now(),
            created_by: created_by.to_string(),
            message,
            protected,
        };

        let key = keys::tag_key(tenant_id, repo_id, tag_name);
        let value = rmp_serde::to_vec(&tag)
            .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

        let cf = cf_handle(&self.db, cf::TAGS)?;
        self.db
            .put_cf(cf, key, value)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        // Query RevisionMeta to find which branch owns this revision
        use raisin_storage::RevisionRepository;
        // Create a temporary RevisionRepository with a placeholder node_id (only used for queries, not allocations)
        let revision_repo = crate::repositories::RevisionRepositoryImpl::new(
            self.db.clone(),
            "tag_query".to_string(),
        );

        match revision_repo
            .get_revision_meta(tenant_id, repo_id, revision)
            .await?
        {
            Some(rev_meta) => {
                let source_branch = &rev_meta.branch;

                tracing::info!(
                    "Copying indexes from branch '{}' at revision {} to tag '{}'",
                    source_branch,
                    revision,
                    tag_name
                );

                // Copy revision-aware indexes from source branch to tag
                self.copy_tag_indexes(tenant_id, repo_id, source_branch, tag_name, *revision)
                    .await?;

                tracing::info!(
                    "Tag '{}' created successfully with indexes copied",
                    tag_name
                );
            }
            None => {
                tracing::warn!(
                    "Revision {} not found in RevisionMeta - tag created without index copying",
                    revision
                );
            }
        }

        Ok(tag)
    }

    async fn get_tag(&self, tenant_id: &str, repo_id: &str, tag_name: &str) -> Result<Option<Tag>> {
        let key = keys::tag_key(tenant_id, repo_id, tag_name);
        let cf = cf_handle(&self.db, cf::TAGS)?;

        match self.db.get_cf(cf, key) {
            Ok(Some(bytes)) => {
                let tag = rmp_serde::from_slice(&bytes).map_err(|e| {
                    raisin_error::Error::storage(format!("Deserialization error: {}", e))
                })?;
                Ok(Some(tag))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(raisin_error::Error::storage(e.to_string())),
        }
    }

    async fn list_tags(&self, tenant_id: &str, repo_id: &str) -> Result<Vec<Tag>> {
        let prefix = keys::KeyBuilder::new()
            .push(tenant_id)
            .push(repo_id)
            .push("tags")
            .build_prefix();

        let cf = cf_handle(&self.db, cf::TAGS)?;
        let prefix_clone = prefix.clone();
        let iter = self.db.prefix_iterator_cf(cf, prefix);

        let mut tags = Vec::new();

        for item in iter {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            // Verify key actually starts with our prefix
            if !key.starts_with(&prefix_clone) {
                break;
            }

            let tag: Tag = rmp_serde::from_slice(&value).map_err(|e| {
                raisin_error::Error::storage(format!("Deserialization error: {}", e))
            })?;
            tags.push(tag);
        }

        // Sort by name
        tags.sort_by(|a, b| a.name.cmp(&b.name));

        Ok(tags)
    }

    async fn delete_tag(&self, tenant_id: &str, repo_id: &str, tag_name: &str) -> Result<bool> {
        let key = keys::tag_key(tenant_id, repo_id, tag_name);
        let cf = cf_handle(&self.db, cf::TAGS)?;

        // Retrieve the tag to check if it's protected
        let tag_opt = self
            .db
            .get_cf(cf, &key)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?
            .map(|bytes| {
                rmp_serde::from_slice::<Tag>(&bytes).map_err(|e| {
                    raisin_error::Error::storage(format!("Deserialization error: {}", e))
                })
            })
            .transpose()?;

        if let Some(tag) = tag_opt {
            // Check if tag is protected
            if tag.protected {
                return Err(raisin_error::Error::Forbidden(format!(
                    "Cannot delete protected tag '{}'",
                    tag_name
                )));
            }

            self.db
                .delete_cf(cf, key)
                .map_err(|e| raisin_error::Error::storage(e.to_string()))?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}
