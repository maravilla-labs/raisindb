//! BFS-based path finding for graph relationship resolution.
//!
//! Implements breadth-first search with early termination for efficient
//! relationship path discovery across workspaces.

use std::collections::{HashSet, VecDeque};

use raisin_error::Result;
use raisin_rel::eval::RelDirection;
use raisin_storage::RelationRepository;

use super::RocksDBGraphResolver;

impl<R: RelationRepository> RocksDBGraphResolver<'_, R> {
    /// Perform BFS to find if a path exists between source and target.
    ///
    /// Uses early termination when the target is found at a valid depth.
    pub(super) async fn bfs_has_path(
        &self,
        source_id: &str,
        target_id: &str,
        relation_types: &[String],
        min_depth: u32,
        max_depth: u32,
        direction: RelDirection,
    ) -> Result<bool> {
        // Optimization: if source == target and min_depth is 0, return true
        if source_id == target_id && min_depth == 0 {
            return Ok(true);
        }

        // Load all relationships of the specified types using global index
        let relation_type_filter = if relation_types.is_empty() {
            None
        } else if relation_types.len() == 1 {
            Some(relation_types[0].as_str())
        } else {
            // Multiple types: load all and filter in memory
            None
        };

        tracing::debug!(
            "BFS path search: {} -> {} (depth {}-{}, direction {:?}, types: {:?})",
            source_id,
            target_id,
            min_depth,
            max_depth,
            direction,
            relation_types
        );

        let all_relations = self
            .relation_repo
            .scan_relations_global(
                raisin_storage::BranchScope::new(self.tenant_id, self.repo_id, self.branch),
                relation_type_filter,
                Some(self.revision),
            )
            .await?;

        tracing::debug!("Loaded {} relationships for BFS", all_relations.len());

        // Build adjacency lists based on direction
        let mut forward_adj: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        let mut reverse_adj: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();

        for (src_workspace, src_id, tgt_workspace, tgt_id, rel) in &all_relations {
            // Filter by relation types if multiple specified
            if !relation_types.is_empty() && !relation_types.iter().any(|t| t == &rel.relation_type)
            {
                continue;
            }

            // Build composite IDs (workspace:node_id) for uniqueness
            let source_key = format!("{}:{}", src_workspace, src_id);
            let target_key = format!("{}:{}", tgt_workspace, tgt_id);

            forward_adj
                .entry(source_key.clone())
                .or_default()
                .push(target_key.clone());
            reverse_adj.entry(target_key).or_default().push(source_key);
        }

        // Determine which adjacency list to use
        let adjacency = match direction {
            RelDirection::Outgoing => &forward_adj,
            RelDirection::Incoming => &reverse_adj,
            RelDirection::Any => {
                // For bidirectional, merge both adjacency lists
                // This is handled in the traversal loop
                &forward_adj // Placeholder, handled specially below
            }
        };

        // BFS queue: (node_id, current_depth)
        let mut queue: VecDeque<(String, u32)> = VecDeque::new();
        let mut visited: HashSet<String> = HashSet::new();

        // Try to find source_id in the graph (with any workspace prefix)
        let source_candidates: Vec<String> = if direction == RelDirection::Incoming {
            // For incoming, start from target and search backwards
            reverse_adj
                .keys()
                .filter(|k| k.ends_with(&format!(":{}", source_id)))
                .cloned()
                .collect()
        } else {
            // For outgoing/any, start from source
            forward_adj
                .keys()
                .filter(|k| k.ends_with(&format!(":{}", source_id)))
                .cloned()
                .collect()
        };

        if source_candidates.is_empty() {
            tracing::debug!("Source node {} not found in graph", source_id);
            return Ok(false);
        }

        // Initialize queue with source node(s)
        for candidate in source_candidates {
            queue.push_back((candidate.clone(), 0));
            visited.insert(candidate);
        }

        // BFS traversal
        while let Some((current, depth)) = queue.pop_front() {
            // Early termination: if we've exceeded max depth, skip
            if depth >= max_depth {
                continue;
            }

            // Get neighbors based on direction
            let neighbors = match direction {
                RelDirection::Outgoing => adjacency.get(&current).cloned().unwrap_or_default(),
                RelDirection::Incoming => reverse_adj.get(&current).cloned().unwrap_or_default(),
                RelDirection::Any => {
                    // Combine both directions
                    let mut combined = forward_adj.get(&current).cloned().unwrap_or_default();
                    combined.extend(reverse_adj.get(&current).cloned().unwrap_or_default());
                    combined
                }
            };

            for neighbor in neighbors {
                // Skip if already visited
                if visited.contains(&neighbor) {
                    continue;
                }

                let next_depth = depth + 1;

                // Check if this is the target
                let is_target = neighbor.ends_with(&format!(":{}", target_id));

                if is_target && next_depth >= min_depth && next_depth <= max_depth {
                    tracing::debug!(
                        "Path found at depth {} (min: {}, max: {})",
                        next_depth,
                        min_depth,
                        max_depth
                    );
                    return Ok(true);
                }

                // Continue BFS if within depth range
                if next_depth < max_depth {
                    queue.push_back((neighbor.clone(), next_depth));
                    visited.insert(neighbor);
                }
            }
        }

        tracing::debug!("No path found after exploring {} nodes", visited.len());
        Ok(false)
    }
}
