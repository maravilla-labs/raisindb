//! User operations: update, delete

use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_models::admin_user::DatabaseAdminUser;
use raisin_replication::Operation;

use super::OperationApplicator;

impl OperationApplicator {
    /// Apply a user update operation
    pub(in crate::replication::application) async fn apply_update_user(
        &self,
        tenant_id: &str,
        user_id: &str,
        user: &DatabaseAdminUser,
        op: &Operation,
    ) -> Result<()> {
        tracing::info!(
            "📥 Applying user update: {}/{} from node {}",
            tenant_id,
            user_id,
            op.cluster_node_id
        );

        // Use username instead of user_id to match AdminUserStore.build_key() format
        let key = keys::admin_user_key(tenant_id, &user.username);
        let cf = cf_handle(&self.db, cf::ADMIN_USERS)?;

        let value = rmp_serde::to_vec(&user)
            .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

        self.db
            .put_cf(cf, key, value)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        tracing::info!("✅ User applied successfully: {}/{}", tenant_id, user_id);
        Ok(())
    }

    /// Apply a user delete operation
    pub(in crate::replication::application) async fn apply_delete_user(
        &self,
        tenant_id: &str,
        user_id: &str,
        op: &Operation,
    ) -> Result<()> {
        tracing::info!(
            "📥 Applying user delete: {}/{} from node {}",
            tenant_id,
            user_id,
            op.cluster_node_id
        );

        let key = keys::admin_user_key(tenant_id, user_id);
        let cf = cf_handle(&self.db, cf::ADMIN_USERS)?;

        self.db
            .delete_cf(cf, key)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        tracing::info!("✅ User deleted successfully: {}/{}", tenant_id, user_id);
        Ok(())
    }
}
