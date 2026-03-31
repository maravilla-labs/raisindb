//! Referential integrity check before deletion.
//!
//! Verifies that no other nodes reference the node being deleted
//! (via Reference properties or Relations).

use super::super::super::helpers::is_tombstone;
use super::super::super::NodeRepositoryImpl;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use std::collections::HashSet;

impl NodeRepositoryImpl {
    pub(in super::super::super) async fn check_delete_safety(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
    ) -> Result<()> {
        let cf_reference = cf_handle(&self.db, cf::REFERENCE_INDEX)?;
        let cf_relation = cf_handle(&self.db, cf::RELATION_INDEX)?;

        let mut referencing_nodes = Vec::new();

        // Get the node to find its path
        let node = match self
            .get_impl(tenant_id, repo_id, branch, workspace, node_id, false)
            .await?
        {
            Some(n) => n,
            None => return Ok(()), // Node doesn't exist, safe to "delete"
        };

        // Check for incoming references (both published and unpublished)
        self.check_incoming_references(
            cf_reference,
            &mut referencing_nodes,
            &node,
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_id,
        )
        .await?;

        // Check for incoming relations
        self.check_incoming_relations(
            cf_relation,
            &mut referencing_nodes,
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_id,
        )
        .await?;

        // If any nodes reference this node, return an error
        if !referencing_nodes.is_empty() {
            return Err(raisin_error::Error::Validation(format!(
                "Cannot delete node '{}': {} other node(s) reference it: [{}]",
                node.path,
                referencing_nodes.len(),
                referencing_nodes.join(", ")
            )));
        }

        Ok(())
    }

    /// Check for incoming references to a node being deleted.
    #[allow(clippy::too_many_arguments)]
    async fn check_incoming_references(
        &self,
        cf_reference: &rocksdb::ColumnFamily,
        referencing_nodes: &mut Vec<String>,
        node: &raisin_models::nodes::Node,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
    ) -> Result<()> {
        for published in [false, true] {
            let ref_prefix = keys::reference_reverse_prefix(
                tenant_id, repo_id, branch, workspace, workspace, &node.path, published,
            );

            let iter = self.db.prefix_iterator_cf(cf_reference, &ref_prefix);

            let mut seen_sources = HashSet::new();

            for item in iter {
                let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

                if !key.starts_with(&ref_prefix) {
                    break;
                }

                if is_tombstone(&value) {
                    continue;
                }

                // Parse key to extract source_node_id
                // Key format: {tenant}\0{repo}\0{branch}\0{workspace}\0ref_rev{_pub}\0{target_workspace}\0{target_path}\0{source_node_id}\0{property_path}\0{~revision}
                let parts: Vec<&[u8]> = key.split(|&b| b == 0).collect();
                if parts.len() >= 8 {
                    let source_node_id = String::from_utf8_lossy(parts[7]).to_string();

                    tracing::debug!(
                        "check_delete_safety: Found reference from source_node_id='{}' to target='{}'",
                        source_node_id, node.path
                    );

                    if source_node_id != node_id && !seen_sources.contains(&source_node_id) {
                        seen_sources.insert(source_node_id.clone());

                        match self
                            .get_impl(
                                tenant_id,
                                repo_id,
                                branch,
                                workspace,
                                &source_node_id,
                                false,
                            )
                            .await
                        {
                            Ok(Some(source_node)) => {
                                referencing_nodes.push(source_node.path.clone());
                            }
                            Ok(None) => {
                                tracing::warn!(
                                    "check_delete_safety: Source node '{}' not found - reference index may be stale",
                                    source_node_id
                                );
                            }
                            Err(e) => {
                                tracing::error!(
                                    "check_delete_safety: Error getting source node '{}': {}",
                                    source_node_id,
                                    e
                                );
                                return Err(e);
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Check for incoming relations to a node being deleted.
    #[allow(clippy::too_many_arguments)]
    async fn check_incoming_relations(
        &self,
        cf_relation: &rocksdb::ColumnFamily,
        referencing_nodes: &mut Vec<String>,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
    ) -> Result<()> {
        let rel_prefix =
            keys::relation_reverse_prefix(tenant_id, repo_id, branch, workspace, node_id);

        let iter = self.db.prefix_iterator_cf(cf_relation, &rel_prefix);

        let mut seen_sources = HashSet::new();

        for item in iter {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            if !key.starts_with(&rel_prefix) {
                break;
            }

            if is_tombstone(&value) {
                continue;
            }

            // Parse key to extract source_node_id
            // Key format: {tenant}\0{repo}\0{branch}\0{workspace}\0rel_rev\0{target_node_id}\0{relation_type}\0{~revision}\0{source_node_id}
            let parts: Vec<&[u8]> = key.split(|&b| b == 0).collect();
            if parts.len() >= 9 {
                let source_node_id = String::from_utf8_lossy(parts[8]).to_string();

                eprintln!(
                    "🔍 check_delete_safety: Found relation from source='{}' to target='{}'",
                    source_node_id, node_id
                );

                if source_node_id != node_id && !seen_sources.contains(&source_node_id) {
                    seen_sources.insert(source_node_id.clone());

                    match self
                        .get_impl(
                            tenant_id,
                            repo_id,
                            branch,
                            workspace,
                            &source_node_id,
                            false,
                        )
                        .await
                    {
                        Ok(Some(source_node)) => {
                            eprintln!("Found source node at path: {}", source_node.path);
                            referencing_nodes.push(source_node.path.clone());
                        }
                        Ok(None) => {
                            eprintln!(
                                "Source node '{}' NOT FOUND - skipping stale index entry",
                                source_node_id
                            );
                        }
                        Err(e) => {
                            eprintln!("Error getting source node '{}': {}", source_node_id, e);
                            return Err(e);
                        }
                    }
                }
            }
        }

        Ok(())
    }
}
