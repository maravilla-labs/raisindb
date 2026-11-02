//! Graph relations of an export (`relations.yaml`, see
//! [`raisin_packages::PackageRelations`]).
//!
//! For every exported node, its OUTGOING relations are written with both
//! endpoints named by `(workspace, path)`; node ids do not survive a package
//! round trip, paths do. A target is written only when the requester can read
//! it and it could be exported itself (allowed workspace and node type):
//! otherwise the relation is left out and counted, so an export never reveals
//! the path of a node its author could not see. Incoming relations from nodes
//! outside the selection are not the selection's content and are not written.

use crate::RocksDBStorage;
use raisin_core::services::package_export_access;
use raisin_error::Result;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::Node;
use raisin_packages::{PackageRelation, RelationEndpoint};
use raisin_storage::{NodeRepository, RelationRepository, Storage, StorageScope};

/// Relations collected while the content is written.
#[derive(Debug, Default)]
pub(super) struct ExportRelations {
    /// Relations to write.
    pub relations: Vec<PackageRelation>,
    /// Relations left out (target gone, unreadable or not exportable).
    pub left_out: u64,
}

impl ExportRelations {
    /// Collect the outgoing relations of `node` (in `workspace`).
    pub(super) async fn collect(
        &mut self,
        storage: &RocksDBStorage,
        (tenant, repo, branch): (&str, &str, &str),
        workspace: &str,
        node: &Node,
        requester: &AuthContext,
    ) -> Result<()> {
        let outgoing = storage
            .relations()
            .get_outgoing_relations(
                StorageScope::new(tenant, repo, branch, workspace),
                &node.id,
                None,
            )
            .await?;
        for rel in outgoing {
            if !super::scope::workspace_allowed(&rel.workspace) {
                self.left_out += 1;
                continue;
            }
            let target = storage
                .nodes()
                .get(
                    StorageScope::new(tenant, repo, branch, &rel.workspace),
                    &rel.target,
                    None,
                )
                .await?
                .filter(|t| super::scope::node_type_allowed(&t.node_type))
                .and_then(|t| {
                    package_export_access::readable_by_requester(
                        t,
                        requester,
                        &rel.workspace,
                        branch,
                    )
                });
            let Some(target) = target else {
                self.left_out += 1;
                continue;
            };
            self.relations.push(PackageRelation {
                from: RelationEndpoint {
                    workspace: workspace.to_string(),
                    path: node.path.clone(),
                },
                to: RelationEndpoint {
                    workspace: rel.workspace.clone(),
                    path: target.path,
                },
                relation_type: rel.relation_type,
                weight: rel.weight,
            });
        }
        Ok(())
    }
}
