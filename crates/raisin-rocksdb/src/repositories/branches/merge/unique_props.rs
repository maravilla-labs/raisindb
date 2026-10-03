//! `unique: true` property names for the node types a merge resolution writes.
//!
//! The UNIQUE_INDEX writers need each NodeType's unique property names, which
//! is a schema read — async — while merge apply's write is sync. So the
//! resolution reads them first, here, and hands them to the write, which
//! goes through the same sync writers the local write path uses.

use super::superseded::Superseded;
use crate::repositories::{BranchRepositoryImpl, NodeTypeRepositoryImpl, RevisionRepositoryImpl};
use raisin_error::Result;
use raisin_storage::NodeTypeRepository;
use std::collections::HashMap;
use std::sync::Arc;

/// `unique: true` property names per NodeType name.
#[derive(Default)]
pub(super) struct UniqueProperties(HashMap<String, Vec<String>>);

impl UniqueProperties {
    /// The names for `node_type`; none when it declares none (or is unknown).
    pub(super) fn of(&self, node_type: &str) -> &[String] {
        self.0.get(node_type).map(Vec::as_slice).unwrap_or(&[])
    }
}

impl BranchRepositoryImpl {
    /// Resolve the unique property names of every node type among `versions`,
    /// as declared on `branch` (the merge target).
    pub(super) async fn unique_properties(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        versions: &[Superseded],
    ) -> Result<UniqueProperties> {
        let node_types = NodeTypeRepositoryImpl::new(
            self.db.clone(),
            Arc::new(RevisionRepositoryImpl::new(
                self.db.clone(),
                "merge-unique-lookup".to_string(),
            )),
            Arc::new(self.clone()),
        );
        let mut out = UniqueProperties::default();
        for version in versions {
            let name = &version.node.node_type;
            if out.0.contains_key(name) {
                continue;
            }
            let names = match node_types
                .get(
                    raisin_storage::BranchScope::new(tenant_id, repo_id, branch),
                    name,
                    None,
                )
                .await?
            {
                Some(node_type) => {
                    crate::repositories::nodes::extract_unique_property_names(&node_type)
                }
                None => Vec::new(),
            };
            out.0.insert(name.clone(), names);
        }
        Ok(out)
    }
}
