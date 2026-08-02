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

    /// Write a user received from another node.
    ///
    /// Unlike [`update_user`](Self::update_user) this does **not** require the
    /// user to exist: `capture_user_operation` emits `OpType::UpdateUser` for
    /// creates as well as updates, so on a node that has never seen the user
    /// the existence check would reject exactly the operation that is supposed
    /// to introduce them.
    ///
    /// Does not capture — this IS the replication apply path, and re-emitting
    /// would loop.
    pub fn put_replicated(&self, user: &DatabaseAdminUser) -> Result<()> {
        let cf = self.db.cf_handle(cf::ADMIN_USERS).ok_or_else(|| {
            raisin_error::Error::Backend("admin_users column family not found".to_string())
        })?;
        let value = rmp_serde::to_vec(user).map_err(|e| {
            raisin_error::Error::Backend(format!("Failed to serialize admin user: {}", e))
        })?;
        self.db
            .put_cf(cf, Self::build_key(&user.tenant_id, &user.username), &value)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))
    }

    /// Delete a user on behalf of another node.
    ///
    /// Idempotent: a delete for a user this node never had is a no-op, not an
    /// error, so a replicated delete cannot wedge the operation queue by
    /// redelivering forever.
    pub fn delete_replicated(&self, tenant_id: &str, username: &str) -> Result<()> {
        let cf = self.db.cf_handle(cf::ADMIN_USERS).ok_or_else(|| {
            raisin_error::Error::Backend("admin_users column family not found".to_string())
        })?;
        self.db
            .delete_cf(cf, Self::build_key(tenant_id, username))
            .map_err(|e| raisin_error::Error::storage(e.to_string()))
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
        if let Some(capture) = self
            .operation_capture
            .as_ref()
            .filter(|c| c.is_enabled())
            .cloned()
        {
            let (tenant, user) = (tenant_id.to_string(), username.to_string());
            let actor = user.clone();
            crate::replication::run_capture("delete_user", async move {
                capture
                    .capture_delete_user(
                        tenant,
                        "system".to_string(),
                        "main".to_string(),
                        user,
                        actor,
                    )
                    .await
            });
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

    /// Check if any admin users exist for a tenant.
    ///
    /// `prefix_iterator_cf` seeks to the prefix position but does NOT filter
    /// by it — `.next()` will return whichever key is lexically next in the
    /// CF, even if it belongs to a different tenant. Without the explicit
    /// `starts_with` bounds check this method returned `true` for every
    /// tenant whose id sorts before any tenant that has users (e.g. anything
    /// before "default"), which broke first-time provisioning for those
    /// tenants. Mirrors the bounds-check pattern used by `list_users`.
    pub fn has_users(&self, tenant_id: &str) -> Result<bool> {
        let cf = self.db.cf_handle(cf::ADMIN_USERS).ok_or_else(|| {
            raisin_error::Error::Backend("admin_users column family not found".to_string())
        })?;

        let prefix = Self::build_tenant_prefix(tenant_id);
        let mut iter = self.db.prefix_iterator_cf(cf, &prefix);

        match iter.next() {
            Some(Ok((key, _))) => Ok(key.starts_with(&prefix)),
            Some(Err(e)) => Err(raisin_error::Error::storage(e.to_string())),
            None => Ok(false),
        }
    }

    /// Helper: Capture a user update/create operation for replication
    fn capture_user_operation(&self, user: &DatabaseAdminUser, _is_create: bool) {
        let Some(capture) = self
            .operation_capture
            .as_ref()
            .filter(|c| c.is_enabled())
            .cloned()
        else {
            return;
        };
        let user_value = serde_json::to_value(user).unwrap_or_else(|_| serde_json::json!({}));
        let (tenant_id, username) = (user.tenant_id.clone(), user.username.clone());
        let actor = username.clone();

        crate::replication::run_capture("update_user", async move {
            capture
                .capture_update_user(
                    tenant_id,
                    "system".to_string(),
                    "main".to_string(),
                    username,
                    user_value,
                    actor,
                )
                .await
        });
    }
}
