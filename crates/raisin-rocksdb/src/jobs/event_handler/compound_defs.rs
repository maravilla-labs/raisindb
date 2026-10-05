//! Keeping the compound/unique definitions cache warm for the replication
//! apply path (plan Phase 8 step 3), off that path.
//!
//! The apply path only PEEKS the cache (`indexing::compound::defs`): a cold
//! write marks the workspace's compound indexes `NotBuilt` and records a
//! build request (`indexing::compound::cold`). This handler drains those
//! requests — re-resolves the branch's definitions and sweeps the workspace's
//! compound builds — so one cold write is followed by warm ones, and the
//! build restores `Ready`.

use super::UnifiedJobEventHandler;
use raisin_storage::BranchScope;

impl UnifiedJobEventHandler {
    /// Re-resolve one branch's definitions (every type on it) and swap them
    /// in atomically — never a cold window for the apply path.
    pub(crate) async fn refresh_compound_definitions(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
    ) {
        use raisin_storage::NodeTypeRepository;
        let db = self.storage.db();
        let scope = BranchScope::new(tenant_id, repo_id, branch);
        let refreshed = async {
            let types = self.storage.node_types.list(scope, None).await?;
            let names: Vec<&str> = types.iter().map(|t| t.name.as_str()).collect();
            crate::indexing::compound::defs::refresh_branch(
                db,
                &self.storage.node_types,
                scope,
                &names,
            )
            .await
        }
        .await;
        if let Err(e) = refreshed {
            tracing::warn!(
                error = %e,
                tenant = %tenant_id,
                repo = %repo_id,
                branch = %branch,
                "could not warm compound index definitions"
            );
        }
    }

    /// Serve every pending cold-definition build request of this database:
    /// re-resolve the branch's definitions from storage — the cold types
    /// included, so a type with no NodeType record is cached as "none" and
    /// stops taking the cold path — then sweep the workspace's builds.
    pub(crate) async fn drain_cold_compound_requests(&self) {
        for ((tenant_id, repo_id, branch, workspace), types) in
            crate::indexing::compound::cold::drain(self.storage.db())
        {
            let types: Vec<&str> = types.iter().map(String::as_str).collect();
            if let Err(e) = crate::indexing::compound::defs::fresh_branch(
                self.storage.db(),
                &self.storage.node_types,
                BranchScope::new(&tenant_id, &repo_id, &branch),
                &types,
            )
            .await
            {
                tracing::warn!(error = %e, "could not warm compound index definitions");
            }
            self.sweep_compound_index_builds_for_workspace(
                &tenant_id, &repo_id, &branch, &workspace, None,
            )
            .await;
        }
    }
}
