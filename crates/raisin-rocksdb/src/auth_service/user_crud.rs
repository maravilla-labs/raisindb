//! Admin user CRUD operations

use super::AuthService;
use raisin_error::Result;
use raisin_models::admin_user::{AdminAccessFlags, DatabaseAdminUser};

impl AuthService {
    /// Create a new admin user
    pub fn create_user(
        &self,
        tenant_id: String,
        username: String,
        email: Option<String>,
        password: String,
        access_flags: AdminAccessFlags,
    ) -> Result<DatabaseAdminUser> {
        // Validate password strength
        Self::validate_password_strength(&password)?;

        // Hash password
        let password_hash = Self::hash_password(&password)?;

        // Generate user ID
        let user_id = uuid::Uuid::new_v4().to_string();

        // Create user
        let user = DatabaseAdminUser::new(user_id, username, email, password_hash, tenant_id);

        // Save to store
        self.store.create_user(&user)?;

        Ok(user)
    }

    /// Create a superadmin user with a generated password
    ///
    /// Returns (user, plain_password) - the plain password should be logged and then discarded
    pub fn create_superadmin(
        &self,
        tenant_id: String,
        username: String,
    ) -> Result<(DatabaseAdminUser, String)> {
        // Generate random password
        let password = Self::generate_password();
        eprintln!("create_superadmin: Generated password: {}", password);

        // Hash password
        let password_hash = Self::hash_password(&password)?;
        eprintln!(
            "create_superadmin: Password hash: {}...",
            &password_hash.chars().take(20).collect::<String>()
        );

        // Immediately verify that the hash can be verified
        let verify_check = Self::verify_password(&password, &password_hash)?;
        eprintln!(
            "create_superadmin: Immediate verification check: {}",
            verify_check
        );
        if !verify_check {
            eprintln!("CRITICAL: Password hash verification failed immediately after hashing!");
        }

        // Generate user ID
        let user_id = uuid::Uuid::new_v4().to_string();

        // Create superadmin user
        let user = DatabaseAdminUser::new_superadmin(user_id, username, password_hash, tenant_id);

        // Save to store
        self.store.create_user(&user)?;

        Ok((user, password))
    }

    /// Create a superadmin user with a specific password
    ///
    /// Returns the created user - password validation is performed
    pub fn create_superadmin_with_password(
        &self,
        tenant_id: String,
        username: String,
        password: String,
    ) -> Result<DatabaseAdminUser> {
        // Validate password strength
        Self::validate_password_strength(&password)?;

        // Hash password
        let password_hash = Self::hash_password(&password)?;

        // Generate user ID
        let user_id = uuid::Uuid::new_v4().to_string();

        // Create superadmin user
        let user = DatabaseAdminUser::new_superadmin(user_id, username, password_hash, tenant_id);

        // Save to store
        self.store.create_user(&user)?;

        Ok(user)
    }

    /// Check if a tenant has any admin users
    pub fn has_users(&self, tenant_id: &str) -> Result<bool> {
        self.store.has_users(tenant_id)
    }

    /// Get a user by username
    pub fn get_user(&self, tenant_id: &str, username: &str) -> Result<Option<DatabaseAdminUser>> {
        self.store.get_user(tenant_id, username)
    }

    /// Get a user by user_id (scans all users in tenant)
    pub fn get_user_by_id(
        &self,
        tenant_id: &str,
        user_id: &str,
    ) -> Result<Option<DatabaseAdminUser>> {
        self.store.get_user_by_id(tenant_id, user_id)
    }

    /// List all users for a tenant
    pub fn list_users(&self, tenant_id: &str) -> Result<Vec<DatabaseAdminUser>> {
        self.store.list_users(tenant_id)
    }

    /// Update a user's information
    pub fn update_user(&self, user: &DatabaseAdminUser) -> Result<()> {
        self.store.update_user(user)
    }

    /// Delete a user
    pub fn delete_user(&self, tenant_id: &str, username: &str) -> Result<()> {
        self.store.delete_user(tenant_id, username)
    }
}
