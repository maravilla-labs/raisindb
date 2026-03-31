//! Graph projection building for background computation.
//!
//! Builds a GraphProjection by scanning nodes and relations within
//! the configured scope from RocksDB storage.

use super::GraphComputeTask;
use crate::graph::scope::ScopeFilter;
use crate::{cf, cf_handle, RocksDBStorage};
use raisin_error::{Error, Result};
use raisin_graph_algorithms::GraphProjection;

impl GraphComputeTask {
    /// Build a graph projection from nodes in scope
    pub(super) async fn build_projection(
        storage: &RocksDBStorage,
        tenant_id: &str,
        repo_id: &str,
        branch_id: &str,
        revision: &str,
        scope: &crate::graph::types::GraphScope,
        max_nodes: usize,
    ) -> Result<GraphProjection> {
        use crate::keys::{extract_revision_from_key, KeyBuilder};
        use crate::repositories::is_node_tombstone as is_tombstone;
        use raisin_hlc::HLC;
        use raisin_storage::{RelationRepository, Storage};
        use std::collections::HashSet;

        let scope_filter = ScopeFilter::from_scope(scope);
        let db = storage.db();
        let max_revision: HLC = revision
            .parse()
            .map_err(|e| Error::storage(format!("Failed to parse revision: {}", e)))?;

        // Step 1: Scan nodes matching scope
        let mut node_ids = HashSet::new();
        let mut node_count = 0;

        // Determine which workspaces to scan
        let workspaces_to_scan = if scope_filter.has_workspace_filters() {
            scope_filter.workspaces().to_vec()
        } else {
            // If no workspace filter, we need to scan all workspaces
            // For now, use a common workspace or scan workspace list
            vec!["default".to_string()]
        };

        for workspace in &workspaces_to_scan {
            if node_count >= max_nodes {
                break;
            }

            // Build prefix for nodes in this workspace
            let prefix = KeyBuilder::new()
                .push(tenant_id)
                .push(repo_id)
                .push(branch_id)
                .push(workspace)
                .push("nodes")
                .build_prefix();

            let cf = cf_handle(db, cf::NODES)?;
            let iter = db.prefix_iterator_cf(cf, &prefix);

            // Track which node IDs we've seen to only process the latest version
            let mut seen_nodes = HashSet::new();

            for item in iter {
                if node_count >= max_nodes {
                    break;
                }

                let (key, value) =
                    item.map_err(|e| Error::storage(format!("Failed to iterate nodes: {}", e)))?;

                // Verify prefix match
                if !key.starts_with(&prefix) {
                    break;
                }

                // Extract node_id and revision from key
                // Key format: {tenant}\0{repo}\0{branch}\0{workspace}\0nodes\0{node_id}\0{~revision}
                let suffix = &key[prefix.len()..];
                let parts: Vec<&[u8]> = suffix.split(|&b| b == 0).collect();
                if parts.len() < 2 {
                    continue;
                }

                let node_id = std::str::from_utf8(parts[0])
                    .map_err(|e| Error::storage(format!("Invalid node_id: {}", e)))?;

                // Skip if we already processed this node (we see newest first)
                if !seen_nodes.insert(node_id.to_string()) {
                    continue;
                }

                // Check revision bound
                let revision = extract_revision_from_key(&key)
                    .map_err(|e| Error::storage(format!("Failed to extract revision: {}", e)))?;
                if revision > max_revision {
                    continue;
                }

                // Skip tombstones
                if is_tombstone(&value) {
                    continue;
                }

                // Deserialize node to check filters
                let storage_node: crate::repositories::StorageNode = rmp_serde::from_slice(&value)
                    .map_err(|e| Error::storage(format!("Failed to deserialize node: {}", e)))?;

                // We need the full path for filtering - reconstruct from storage_node
                // For now, use a simple path (storage_node.path or reconstruct if needed)
                let node_path = format!("{}/{}", workspace, storage_node.name);
                let node_type = &storage_node.node_type;

                // Apply scope filters
                if scope_filter.matches(&node_path, node_type, workspace) {
                    node_ids.insert(storage_node.id.clone());
                    node_count += 1;
                }
            }
        }

        // Step 2: Scan relations matching scope
        let relation_type_filter = if scope_filter.has_relation_type_filters() {
            // For multiple relation types, we'd need to scan multiple times or filter in memory
            // For now, if there's exactly one type, use it as filter
            if scope_filter.relation_types().len() == 1 {
                Some(scope_filter.relation_types()[0].as_str())
            } else {
                None // Scan all, filter in memory
            }
        } else {
            None // No filter, include all relations
        };

        let all_relations = storage
            .relations()
            .scan_relations_global(
                raisin_storage::BranchScope::new(tenant_id, repo_id, branch_id),
                relation_type_filter,
                Some(&max_revision),
            )
            .await?;

        // Step 3: Build edge list, filtering to nodes in scope
        let mut edges = Vec::new();
        for (_src_workspace, src_id, _tgt_workspace, tgt_id, rel) in all_relations {
            // Filter by relation type if multiple types specified
            if scope_filter.has_relation_type_filters()
                && !scope_filter.matches_relation_type(&rel.relation_type)
            {
                continue;
            }

            // Only include edges where both endpoints are in our node set
            if node_ids.contains(&src_id) && node_ids.contains(&tgt_id) {
                edges.push((src_id, tgt_id));
            }
        }

        // Convert HashSet to Vec for GraphProjection
        let nodes: Vec<String> = node_ids.into_iter().collect();

        tracing::debug!(
            "Built graph projection: {} nodes, {} edges",
            nodes.len(),
            edges.len()
        );

        Ok(GraphProjection::from_parts(nodes, edges))
    }
}
