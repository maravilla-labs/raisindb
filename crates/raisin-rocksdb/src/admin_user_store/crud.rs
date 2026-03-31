//! CRUD operations for admin users.

use super::AdminUserStore;
use crate::cf;
use raisin_error::Result;
use raisin_models::admin_user::DatabaseAdminUser;

impl AdminUserStore {
    /// Create a new admin user
    pub fn create_user(&self, user: &DatabaseAdminUser) -> Result<()> {
        eprintln!(
            "🔍 create_user() called: tenant={}, username={}",
            user.tenant_id, user.username
        );

        let cf = self.db.cf_handle(cf::ADMIN_USERS).ok_or_else(|| {
            eprintln!("❌ ADMIN_USERS column family not found during create!");
            raisin_error::Error::Backend("admin_users column family not found".to_string())
        })?;

        let key = Self::build_key(&user.tenant_id, &user.username);
        eprintln!(
            "🔍 Creating user with key: {:?}",
            String::from_utf8_lossy(&key)
        );

        // Check if user already exists
        if self
            .db
            .get_cf(cf, &key)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?
            .is_some()
        {
            eprintln!("❌ User already exists");
            return Err(raisin_error::Error::Conflict(format!(
                "User '{}' already exists in tenant '{}'",
                user.username, user.tenant_id
            )));
        }

        // Serialize user to MessagePack
        let value = rmp_serde::to_vec(user).map_err(|e| {
            eprintln!("❌ Failed to serialize user: {}", e);
            raisin_error::Error::Backend(format!("Failed to serialize admin user: {}", e))
        })?;

        eprintln!("🔍 Serialized user to {} bytes", value.len());

        // Store in database
        self.db.put_cf(cf, &key, &value).map_err(|e| {
            eprintln!("❌ Failed to write to RocksDB: {}", e);
            raisin_error::Error::storage(e.to_string())
        })?;

        eprintln!("✅ User successfully written to RocksDB");

        // Capture operation for replication
        self.capture_user_operation(user, /* is_create */ true);

        Ok(())
    }

    /// Get an admin user by username and tenant
    pub fn get_user(&self, tenant_id: &str, username: &str) -> Result<Option<DatabaseAdminUser>> {
        eprintln!(
            "🔍 get_user() called: tenant={}, username={}",
            tenant_id, username
        );

        let cf = self.db.cf_handle(cf::ADMIN_USERS).ok_or_else(|| {
            eprintln!("❌ ADMIN_USERS column family not found!");
            raisin_error::Error::Backend("admin_users column family not found".to_string())
        })?;

        let key = Self::build_key(tenant_id, username);
        eprintln!("🔍 Searching with key: {:?}", String::from_utf8_lossy(&key));

        match self
            .db
            .get_cf(cf, &key)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?
        {
            Some(value) => {
                eprintln!("✅ Found user data in RocksDB ({} bytes)", value.len());
                let user: DatabaseAdminUser = rmp_serde::from_slice(&value).map_err(|e| {
                    eprintln!("❌ Failed to deserialize user: {}", e);
                    raisin_error::Error::Backend(format!("Failed to deserialize admin user: {}", e))
                })?;
                eprintln!("✅ Deserialized user: {}", user.username);
                Ok(Some(user))
            }
            None => {
                eprintln!("❌ No user found in RocksDB for this key");
                Ok(None)
            }
        }
    }

    /// Get an admin user by user_id (scans all users in tenant)
    pub fn get_user_by_id(
        &self,
        tenant_id: &str,
        user_id: &str,
    ) -> Result<Option<DatabaseAdminUser>> {
        eprintln!(
            "🔍 get_user_by_id() called: tenant={}, user_id={}",
            tenant_id, user_id
        );

        let cf = self.db.cf_handle(cf::ADMIN_USERS).ok_or_else(|| {
            eprintln!("❌ ADMIN_USERS column family not found!");
            raisin_error::Error::Backend("admin_users column family not found".to_string())
        })?;

        let prefix = Self::build_tenant_prefix(tenant_id);
        let iter = self.db.prefix_iterator_cf(cf, &prefix);

        for item in iter {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            // Check if key starts with our prefix
            if !key.starts_with(&prefix) {
                break;
            }

            let user: DatabaseAdminUser = rmp_serde::from_slice(&value).map_err(|e| {
                raisin_error::Error::Backend(format!("Failed to deserialize admin user: {}", e))
            })?;

            if user.user_id == user_id {
                eprintln!("✅ Found user by user_id: {}", user.username);
                return Ok(Some(user));
            }
        }

        eprintln!("❌ No user found with user_id: {}", user_id);
        Ok(None)
    }

    /// Update an existing admin user
    pub fn update_user(&self, user: &DatabaseAdminUser) -> Result<()> {
        let cf = self.db.cf_handle(cf::ADMIN_USERS).ok_or_else(|| {
            raisin_error::Error::Backend("admin_users column family not found".to_string())
        })?;

        let key = Self::build_key(&user.tenant_id, &user.username);

        // Check if user exists
        if self
            .db
            .get_cf(cf, &key)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?
            .is_none()
        {
            return Err(raisin_error::Error::NotFound(format!(
                "User '{}' not found in tenant '{}'",
                user.username, user.tenant_id
            )));
        }

        // Serialize and update
        let value = rmp_serde::to_vec(user).map_err(|e| {
            raisin_error::Error::Backend(format!("Failed to serialize admin user: {}", e))
        })?;

        self.db
            .put_cf(cf, &key, &value)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        // Capture operation for replication
        self.capture_user_operation(user, /* is_create */ false);

        Ok(())
    }

    /// Delete an admin user
    pub fn delete_user(&self, tenant_id: &str, username: &str) -> Result<()> {
        let cf = self.db.cf_handle(cf::ADMIN_USERS).ok_or_else(|| {
            raisin_error::Error::Backend("admin_users column family not found".to_string())
        })?;

        let key = Self::build_key(tenant_id, username);

        // Check if user exists before deleting
        if self
            .db
            .get_cf(cf, &key)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?
            .is_none()
        {
            return Err(raisin_error::Error::NotFound(format!(
                "User '{}' not found in tenant '{}'",
                username, tenant_id
            )));
        }

        self.db
            .delete_cf(cf, &key)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        // Capture operation for replication
        if let Some(ref operation_capture) = self.operation_capture {
            if operation_capture.is_enabled() {
                let _ = tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(async {
                        operation_capture
                            .capture_delete_user(
                                tenant_id.to_string(),
                                "system".to_string(),
                                "main".to_string(),
                                username.to_string(),
                                username.to_string(), // actor
                            )
                            .await
                    })
                });
            }
        }

        Ok(())
    }

    /// List all admin users for a tenant
    pub fn list_users(&self, tenant_id: &str) -> Result<Vec<DatabaseAdminUser>> {
        let cf = self.db.cf_handle(cf::ADMIN_USERS).ok_or_else(|| {
            raisin_error::Error::Backend("admin_users column family not found".to_string())
        })?;

        let prefix = Self::build_tenant_prefix(tenant_id);
        let mut users = Vec::new();

        let iter = self.db.prefix_iterator_cf(cf, &prefix);

        for item in iter {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            // Check if key still matches our prefix (iterator might go beyond)
            if !key.starts_with(&prefix) {
                break;
            }

            let user: DatabaseAdminUser = rmp_serde::from_slice(&value).map_err(|e| {
                raisin_error::Error::Backend(format!("Failed to deserialize admin user: {}", e))
            })?;

            users.push(user);
        }

        Ok(users)
    }

    /// Check if any admin users exist for a tenant
    pub fn has_users(&self, tenant_id: &str) -> Result<bool> {
        let cf = self.db.cf_handle(cf::ADMIN_USERS).ok_or_else(|| {
            raisin_error::Error::Backend("admin_users column family not found".to_string())
        })?;

        let prefix = Self::build_tenant_prefix(tenant_id);
        let mut iter = self.db.prefix_iterator_cf(cf, &prefix);

        Ok(iter.next().is_some())
    }

    /// Helper: Capture a user update/create operation for replication
    fn capture_user_operation(&self, user: &DatabaseAdminUser, _is_create: bool) {
        if let Some(ref operation_capture) = self.operation_capture {
            if operation_capture.is_enabled() {
                let user_value =
                    serde_json::to_value(user).unwrap_or_else(|_| serde_json::json!({}));

                let _ = tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(async {
                        operation_capture
                            .capture_update_user(
                                user.tenant_id.clone(),
                                "system".to_string(),
                                "main".to_string(),
                                user.username.clone(),
                                user_value,
                                user.username.clone(),
                            )
                            .await
                    })
                });
            }
        }
    }
}
