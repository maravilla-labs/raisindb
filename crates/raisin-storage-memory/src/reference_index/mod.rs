//! In-memory reference indexing implementation
//!
//! Provides O(1) reference lookups using HashMap-based indexes.
//! Maintains bidirectional indexes (forward and reverse) with separate draft and published spaces.

use crate::index_types::{ForwardReferenceIndex, ReverseReferenceIndex};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::{
    extract_references, group_references_by_target, reference_target_key, PropertyValue,
    RaisinReference,
};
use raisin_storage::scope::StorageScope;
use raisin_storage::ReferenceIndexRepository;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// In-memory reference index repository
///
/// Uses nested HashMaps for O(1) lookups:
/// - Forward index: composite_key -> node_id -> Vec<(property_path, RaisinReference)>
/// - Reverse index: composite_key -> target_key -> Vec<(source_node_id, property_path)>
/// - Maintains separate draft and published indexes
#[derive(Clone)]
pub struct InMemoryReferenceIndexRepo {
    // Forward index (source -> targets):
    // composite_key -> node_id -> Vec<(property_path, RaisinReference)>
    draft_forward: ForwardReferenceIndex,
    published_forward: ForwardReferenceIndex,

    // Reverse index (target -> sources):
    // composite_key -> target_key -> Vec<(source_node_id, property_path)>
    // target_key format: "{target_workspace}:{target_path}"
    draft_reverse: ReverseReferenceIndex,
    published_reverse: ReverseReferenceIndex,
}

impl InMemoryReferenceIndexRepo {
    /// Create a new in-memory reference index repository
    pub fn new() -> Self {
        Self {
            draft_forward: Arc::new(RwLock::new(HashMap::new())),
            published_forward: Arc::new(RwLock::new(HashMap::new())),
            draft_reverse: Arc::new(RwLock::new(HashMap::new())),
            published_reverse: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create composite key for repository isolation (branchless)
    fn composite_key(tenant_id: &str, repo_id: &str, workspace: &str) -> String {
        format!("{}/{}/{}", tenant_id, repo_id, workspace)
    }
}

impl Default for InMemoryReferenceIndexRepo {
    fn default() -> Self {
        Self::new()
    }
}

impl ReferenceIndexRepository for InMemoryReferenceIndexRepo {
    async fn index_references(
        &self,
        scope: StorageScope<'_>,
        node_id: &str,
        properties: &HashMap<String, PropertyValue>,
        _revision: &HLC,
        is_published: bool,
    ) -> Result<()> {
        let references = extract_references(properties);

        if references.is_empty() {
            return Ok(());
        }

        let (forward_index, reverse_index) = if is_published {
            (&self.published_forward, &self.published_reverse)
        } else {
            (&self.draft_forward, &self.draft_reverse)
        };

        let key = Self::composite_key(scope.tenant_id, scope.repo_id, scope.workspace);

        // Add to forward index
        {
            let mut forward = forward_index.write().unwrap();
            forward
                .entry(key.clone())
                .or_default()
                .insert(node_id.to_string(), references.clone());
        }

        // Add to reverse index
        {
            let mut reverse = reverse_index.write().unwrap();
            let workspace_reverse = reverse.entry(key).or_default();

            for (prop_path, reference) in references {
                let target_key = reference_target_key(&reference);

                workspace_reverse
                    .entry(target_key)
                    .or_default()
                    .push((node_id.to_string(), prop_path));
            }
        }

        Ok(())
    }

    async fn unindex_references(
        &self,
        scope: StorageScope<'_>,
        node_id: &str,
        _properties: &HashMap<String, PropertyValue>,
        _revision: &HLC,
    ) -> Result<()> {
        let key = Self::composite_key(scope.tenant_id, scope.repo_id, scope.workspace);

        // Remove from both draft and published indexes
        for (forward_index, reverse_index) in [
            (&self.draft_forward, &self.draft_reverse),
            (&self.published_forward, &self.published_reverse),
        ] {
            // Get references from forward index before removing
            let references_opt = {
                let forward = forward_index.read().unwrap();
                forward
                    .get(&key)
                    .and_then(|nodes| nodes.get(node_id))
                    .cloned()
            };

            // Remove from forward index
            {
                let mut forward = forward_index.write().unwrap();
                if let Some(workspace_forward) = forward.get_mut(&key) {
                    workspace_forward.remove(node_id);
                }
            }

            // Remove from reverse index
            if let Some(references) = references_opt {
                let mut reverse = reverse_index.write().unwrap();
                if let Some(workspace_reverse) = reverse.get_mut(&key) {
                    for (_, reference) in references {
                        let target_key = reference_target_key(&reference);

                        if let Some(sources) = workspace_reverse.get_mut(&target_key) {
                            sources.retain(|(source_id, _)| source_id != node_id);

                            // Clean up empty entries
                            if sources.is_empty() {
                                workspace_reverse.remove(&target_key);
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }

    async fn update_reference_publish_status(
        &self,
        scope: StorageScope<'_>,
        node_id: &str,
        properties: &HashMap<String, PropertyValue>,
        revision: &HLC,
        is_published: bool,
    ) -> Result<()> {
        // Remove existing references from both index spaces
        self.unindex_references(scope, node_id, properties, revision)
            .await?;

        // Add to new index space
        self.index_references(scope, node_id, properties, revision, is_published)
            .await
    }

    async fn find_referencing_nodes(
        &self,
        scope: StorageScope<'_>,
        target_workspace: &str,
        target_path: &str,
        published_only: bool,
    ) -> Result<Vec<(String, String)>> {
        let reverse_index = if published_only {
            &self.published_reverse
        } else {
            &self.draft_reverse
        };

        let reverse = reverse_index.read().unwrap();
        let key = Self::composite_key(scope.tenant_id, scope.repo_id, scope.workspace);
        let target_key = format!("{}:{}", target_workspace, target_path);

        let results = reverse
            .get(&key)
            .and_then(|workspace_reverse| workspace_reverse.get(&target_key))
            .cloned()
            .unwrap_or_default();

        Ok(results)
    }

    async fn get_node_references(
        &self,
        scope: StorageScope<'_>,
        node_id: &str,
        published_only: bool,
    ) -> Result<Vec<(String, RaisinReference)>> {
        let forward_index = if published_only {
            &self.published_forward
        } else {
            &self.draft_forward
        };

        let forward = forward_index.read().unwrap();
        let key = Self::composite_key(scope.tenant_id, scope.repo_id, scope.workspace);

        let results = forward
            .get(&key)
            .and_then(|nodes| nodes.get(node_id))
            .cloned()
            .unwrap_or_default();

        Ok(results)
    }

    async fn get_unique_references(
        &self,
        scope: StorageScope<'_>,
        node_id: &str,
        published_only: bool,
    ) -> Result<HashMap<String, (Vec<String>, RaisinReference)>> {
        let references = self
            .get_node_references(scope, node_id, published_only)
            .await?;

        Ok(group_references_by_target(references))
    }
}

#[cfg(test)]
mod tests;
