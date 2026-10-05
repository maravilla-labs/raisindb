//! What a compound build writes (`build.rs`): per node type, plus the
//! workspace's own declarations, which every node carries (plan Phase 13e).

use raisin_models::nodes::properties::schema::CompoundIndexDefinition;
use std::collections::HashMap;

/// The declarations a build writes: per node type (types absent: none),
/// plus the ones EVERY node of the workspace carries (the workspace's own
/// indexes, plan Phase 13e).
#[derive(Debug, Default, Clone)]
pub struct Wanted {
    /// Per node type.
    pub by_type: HashMap<String, Vec<CompoundIndexDefinition>>,
    /// Every node, whatever its type.
    pub every_type: Vec<CompoundIndexDefinition>,
}

impl Wanted {
    /// A build of workspace-owned declarations only.
    pub fn every_type(defs: Vec<CompoundIndexDefinition>) -> Self {
        Self {
            by_type: HashMap::new(),
            every_type: defs,
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.every_type.is_empty() && self.by_type.values().all(Vec::is_empty)
    }

    pub(super) fn wants(&self, node_type: &str) -> bool {
        !self.every_type.is_empty() || self.by_type.contains_key(node_type)
    }

    pub(super) fn defs_for<'s>(
        &'s self,
        node_type: &str,
    ) -> impl Iterator<Item = &'s CompoundIndexDefinition> {
        self.by_type
            .get(node_type)
            .map_or(&[][..], Vec::as_slice)
            .iter()
            .chain(&self.every_type)
    }
}

impl FromIterator<(String, Vec<CompoundIndexDefinition>)> for Wanted {
    fn from_iter<I: IntoIterator<Item = (String, Vec<CompoundIndexDefinition>)>>(iter: I) -> Self {
        Self {
            by_type: iter.into_iter().collect(),
            every_type: Vec::new(),
        }
    }
}
