//! The schema-driven index definitions (compound declarations and `unique:
//! true` property names) for the node types a merge resolution writes.
//!
//! Both are a schema read — async — while merge apply's write is sync. So the
//! resolution resolves them first, here, through the one definitions cache
//! (`indexing::compound::defs`), and hands them to the write, which goes
//! through the same sync writers the local write path uses.

use super::superseded::Superseded;
use crate::indexing::compound::DefsSet;
use crate::repositories::{BranchRepositoryImpl, NodeTypeRepositoryImpl, RevisionRepositoryImpl};
use raisin_error::Result;
use std::sync::Arc;

/// Definitions per NodeType name, as declared on the merge target.
#[derive(Default)]
pub(super) struct SchemaDefs(DefsSet);

impl SchemaDefs {
    /// The unique property names of `node_type`; none when it declares none.
    pub(super) fn of(&self, node_type: &str) -> &[String] {
        self.0.unique(node_type)
    }

    /// Every resolved definition (compound writer input).
    pub(super) fn defs(&self) -> &DefsSet {
        &self.0
    }
}

impl BranchRepositoryImpl {
    /// Resolve the definitions of every node type among `versions` (a
    /// resolution writes one of those versions' types), as declared on
    /// `branch` (the merge target).
    pub(super) async fn schema_definitions(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        versions: &[Superseded],
    ) -> Result<SchemaDefs> {
        let node_types = NodeTypeRepositoryImpl::new(
            self.db.clone(),
            Arc::new(RevisionRepositoryImpl::new(
                self.db.clone(),
                "merge-unique-lookup".to_string(),
            )),
            Arc::new(self.clone()),
        );
        let types: Vec<&str> = versions.iter().map(|v| v.node.node_type.as_str()).collect();
        let defs = DefsSet::resolve(
            &self.db,
            &node_types,
            raisin_storage::BranchScope::new(tenant_id, repo_id, branch),
            &types,
        )
        .await?;
        Ok(SchemaDefs(defs))
    }
}
