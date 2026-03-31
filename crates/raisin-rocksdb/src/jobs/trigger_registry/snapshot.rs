//! Immutable trigger registry snapshot with inverted indexes

use super::types::CachedTrigger;
use std::collections::{HashMap, HashSet};
use std::time::Instant;

/// Immutable snapshot of trigger registry
///
/// Once created, this snapshot is never modified. Updates create a new
/// snapshot and atomically swap it in.
pub(crate) struct TriggerRegistrySnapshot {
    /// All triggers indexed by ID
    pub(super) triggers: HashMap<String, CachedTrigger>,

    /// Inverted indexes for O(1) lookups
    /// Maps workspace -> set of trigger IDs
    by_workspace: HashMap<String, HashSet<String>>,
    /// Maps node_type -> set of trigger IDs
    by_node_type: HashMap<String, HashSet<String>>,
    /// Maps event_kind -> set of trigger IDs
    by_event_kind: HashMap<String, HashSet<String>>,

    /// Triggers with no workspace filter (match all workspaces)
    wildcard_workspace: HashSet<String>,
    /// Triggers with no node_type filter (match all types)
    wildcard_node_type: HashSet<String>,

    /// Quick-reject sets - if workspace/type not in these sets, no matches possible
    indexed_workspaces: HashSet<String>,
    indexed_node_types: HashSet<String>,

    /// Metadata
    pub(super) loaded_at: Instant,
    pub(super) version: u64,
}

impl TriggerRegistrySnapshot {
    /// Create an empty snapshot
    pub(super) fn empty() -> Self {
        Self {
            triggers: HashMap::new(),
            by_workspace: HashMap::new(),
            by_node_type: HashMap::new(),
            by_event_kind: HashMap::new(),
            wildcard_workspace: HashSet::new(),
            wildcard_node_type: HashSet::new(),
            indexed_workspaces: HashSet::new(),
            indexed_node_types: HashSet::new(),
            loaded_at: Instant::now(),
            version: 0,
        }
    }

    /// Build inverted indexes from triggers
    pub(super) fn build_indexes(triggers: Vec<CachedTrigger>, version: u64) -> Self {
        let mut snapshot = Self {
            triggers: HashMap::new(),
            by_workspace: HashMap::new(),
            by_node_type: HashMap::new(),
            by_event_kind: HashMap::new(),
            wildcard_workspace: HashSet::new(),
            wildcard_node_type: HashSet::new(),
            indexed_workspaces: HashSet::new(),
            indexed_node_types: HashSet::new(),
            loaded_at: Instant::now(),
            version,
        };

        for trigger in triggers {
            let trigger_id = trigger.id.clone();

            // Index by workspace
            if let Some(workspaces) = &trigger.filters.workspaces {
                for workspace in workspaces {
                    if workspace == "*" {
                        snapshot.wildcard_workspace.insert(trigger_id.clone());
                    } else {
                        snapshot
                            .by_workspace
                            .entry(workspace.clone())
                            .or_default()
                            .insert(trigger_id.clone());
                        snapshot.indexed_workspaces.insert(workspace.clone());
                    }
                }
            } else {
                // No workspace filter = matches all
                snapshot.wildcard_workspace.insert(trigger_id.clone());
            }

            // Index by node_type
            if let Some(node_types) = &trigger.filters.node_types {
                for node_type in node_types {
                    snapshot
                        .by_node_type
                        .entry(node_type.clone())
                        .or_default()
                        .insert(trigger_id.clone());
                    snapshot.indexed_node_types.insert(node_type.clone());
                }
            } else {
                // No node_type filter = matches all
                snapshot.wildcard_node_type.insert(trigger_id.clone());
            }

            // Index by event_kind
            for event_kind in &trigger.event_kinds {
                snapshot
                    .by_event_kind
                    .entry(event_kind.clone())
                    .or_default()
                    .insert(trigger_id.clone());
            }

            // Store trigger
            snapshot.triggers.insert(trigger_id, trigger);
        }

        snapshot
    }

    /// Check if there could be any matches for this workspace and node_type
    pub(super) fn could_have_matches(&self, workspace: &str, node_type: &str) -> bool {
        // Fail-open: if registry is unloaded (version 0, no triggers), allow all events
        // This ensures triggers work before first cache population
        if self.version == 0 && self.triggers.is_empty() {
            return true;
        }

        // Wildcards always match
        if !self.wildcard_workspace.is_empty() || !self.wildcard_node_type.is_empty() {
            return true;
        }

        // Check if workspace is indexed
        let has_workspace = self.indexed_workspaces.contains(workspace);
        // Check if node_type is indexed
        let has_node_type = self.indexed_node_types.contains(node_type);

        // If either is indexed, we might have matches
        has_workspace || has_node_type
    }

    /// Get candidate triggers for an event
    ///
    /// Returns triggers that might match based on workspace, node_type, and event_kind.
    /// Further filtering (paths, properties) must be done by the caller.
    pub(super) fn get_candidates(
        &self,
        workspace: &str,
        node_type: &str,
        event_kind: &str,
    ) -> Vec<CachedTrigger> {
        let mut candidate_ids = HashSet::new();

        // Add wildcards (they match everything)
        candidate_ids.extend(self.wildcard_workspace.iter().cloned());
        candidate_ids.extend(self.wildcard_node_type.iter().cloned());

        // Add triggers matching workspace
        if let Some(ids) = self.by_workspace.get(workspace) {
            candidate_ids.extend(ids.iter().cloned());
        }

        // Add triggers matching node_type
        if let Some(ids) = self.by_node_type.get(node_type) {
            candidate_ids.extend(ids.iter().cloned());
        }

        // Filter by event_kind
        let event_kind_ids = self.by_event_kind.get(event_kind);

        candidate_ids
            .into_iter()
            .filter(|id| {
                // Must match event_kind
                event_kind_ids.is_some_and(|ids| ids.contains(id))
            })
            .filter_map(|id| self.triggers.get(&id).cloned())
            .collect()
    }
}
