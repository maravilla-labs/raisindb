//! ACL (Access Control List) statement AST types
//!
//! Defines the abstract syntax tree for SQL access control extensions
//! including roles, groups, users, grants, and security configuration.

use std::fmt;

// ---------------------------------------------------------------------------
// Top-level ACL statement enum
// ---------------------------------------------------------------------------

/// All ACL statement variants
#[derive(Debug, Clone)]
pub enum AclStatement {
    /// CREATE ROLE statement
    CreateRole(CreateRole),
    /// ALTER ROLE statement
    AlterRole(AlterRole),
    /// DROP ROLE statement
    DropRole(DropRole),
    /// SHOW ROLES statement
    ShowRoles(ShowRoles),
    /// DESCRIBE ROLE statement
    DescribeRole(DescribeRole),

    /// CREATE GROUP statement
    CreateGroup(CreateGroup),
    /// ALTER GROUP statement
    AlterGroup(AlterGroup),
    /// DROP GROUP statement
    DropGroup(DropGroup),
    /// SHOW GROUPS statement
    ShowGroups(ShowGroups),
    /// DESCRIBE GROUP statement
    DescribeGroup(DescribeGroup),

    /// CREATE USER statement
    CreateUser(CreateUser),
    /// ALTER USER statement
    AlterUser(AlterUser),
    /// DROP USER statement
    DropUser(DropUser),
    /// SHOW USERS statement
    ShowUsers(ShowUsers),
    /// DESCRIBE USER statement
    DescribeUser(DescribeUser),

    /// GRANT statement
    Grant(Grant),
    /// REVOKE statement
    Revoke(Revoke),

    /// ALTER SECURITY CONFIG statement
    AlterSecurityConfig(AlterSecurityConfig),
    /// SHOW SECURITY CONFIG statement
    ShowSecurityConfig(ShowSecurityConfig),

    /// SHOW PERMISSIONS FOR statement
    ShowPermissionsFor(ShowPermissionsFor),
    /// SHOW EFFECTIVE ROLES FOR statement
    ShowEffectiveRolesFor(ShowEffectiveRolesFor),
}

impl AclStatement {
    /// Get the operation kind as a string
    pub fn operation(&self) -> &'static str {
        match self {
            AclStatement::CreateRole(_) => "CREATE ROLE",
            AclStatement::AlterRole(_) => "ALTER ROLE",
            AclStatement::DropRole(_) => "DROP ROLE",
            AclStatement::ShowRoles(_) => "SHOW ROLES",
            AclStatement::DescribeRole(_) => "DESCRIBE ROLE",

            AclStatement::CreateGroup(_) => "CREATE GROUP",
            AclStatement::AlterGroup(_) => "ALTER GROUP",
            AclStatement::DropGroup(_) => "DROP GROUP",
            AclStatement::ShowGroups(_) => "SHOW GROUPS",
            AclStatement::DescribeGroup(_) => "DESCRIBE GROUP",

            AclStatement::CreateUser(_) => "CREATE USER",
            AclStatement::AlterUser(_) => "ALTER USER",
            AclStatement::DropUser(_) => "DROP USER",
            AclStatement::ShowUsers(_) => "SHOW USERS",
            AclStatement::DescribeUser(_) => "DESCRIBE USER",

            AclStatement::Grant(_) => "GRANT",
            AclStatement::Revoke(_) => "REVOKE",

            AclStatement::AlterSecurityConfig(_) => "ALTER SECURITY CONFIG",
            AclStatement::ShowSecurityConfig(_) => "SHOW SECURITY CONFIG",

            AclStatement::ShowPermissionsFor(_) => "SHOW PERMISSIONS FOR",
            AclStatement::ShowEffectiveRolesFor(_) => "SHOW EFFECTIVE ROLES FOR",
        }
    }

    /// Returns `true` if this statement only reads data (SHOW, DESCRIBE).
    /// Returns `false` for mutations (CREATE, ALTER, DROP, GRANT, REVOKE).
    pub fn is_read_only(&self) -> bool {
        matches!(
            self,
            AclStatement::ShowRoles(_)
                | AclStatement::DescribeRole(_)
                | AclStatement::ShowGroups(_)
                | AclStatement::DescribeGroup(_)
                | AclStatement::ShowUsers(_)
                | AclStatement::DescribeUser(_)
                | AclStatement::ShowSecurityConfig(_)
                | AclStatement::ShowPermissionsFor(_)
                | AclStatement::ShowEffectiveRolesFor(_)
        )
    }
}

// ---------------------------------------------------------------------------
// Operation enum (shared permission operations)
// ---------------------------------------------------------------------------

/// Permission operations that can be granted or denied
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Operation {
    /// Create new nodes
    Create,
    /// Read existing nodes
    Read,
    /// Update existing nodes
    Update,
    /// Delete nodes
    Delete,
    /// Translate nodes between locales
    Translate,
    /// Create relationships between nodes
    Relate,
    /// Remove relationships between nodes
    Unrelate,
}

impl fmt::Display for Operation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Operation::Create => write!(f, "CREATE"),
            Operation::Read => write!(f, "READ"),
            Operation::Update => write!(f, "UPDATE"),
            Operation::Delete => write!(f, "DELETE"),
            Operation::Translate => write!(f, "TRANSLATE"),
            Operation::Relate => write!(f, "RELATE"),
            Operation::Unrelate => write!(f, "UNRELATE"),
        }
    }
}

// ---------------------------------------------------------------------------
// PermissionGrant (shared struct for role permissions)
// ---------------------------------------------------------------------------

/// A single permission grant within a role definition
///
/// Specifies which operations are allowed on which resources,
/// with optional filtering by workspace, path, branch, node type, and fields.
#[derive(Debug, Clone)]
pub struct PermissionGrant {
    /// Operations permitted (e.g., CREATE, READ, UPDATE)
    pub operations: Vec<Operation>,
    /// Optional workspace glob pattern (e.g., "site-*")
    pub workspace: Option<String>,
    /// Path glob pattern (e.g., "/content/**")
    pub path: String,
    /// Optional branch glob pattern (e.g., "release/*")
    pub branch_pattern: Option<String>,
    /// Optional node type filter (e.g., ["article", "page"])
    pub node_types: Option<Vec<String>>,
    /// Optional field whitelist — only these fields are accessible
    pub fields: Option<Vec<String>>,
    /// Optional field blacklist — all fields except these are accessible
    pub except_fields: Option<Vec<String>>,
    /// Optional REL condition expression for row-level filtering
    pub condition: Option<String>,
}

// ---------------------------------------------------------------------------
// Role statements
// ---------------------------------------------------------------------------

/// CREATE ROLE statement
///
/// Defines a new role with optional inheritance and permissions.
#[derive(Debug, Clone)]
pub struct CreateRole {
    /// Unique role identifier
    pub role_id: String,
    /// Optional human-readable description
    pub description: Option<String>,
    /// Roles this role inherits from
    pub inherits: Vec<String>,
    /// Permission grants defined inline
    pub permissions: Vec<PermissionGrant>,
}

/// ALTER ROLE statement
///
/// Modifies an existing role's permissions, inheritance, or description.
#[derive(Debug, Clone)]
pub struct AlterRole {
    /// Role to alter
    pub role_id: String,
    /// The alteration action to perform
    pub action: AlterRoleAction,
}

/// Actions that can be performed when altering a role
#[derive(Debug, Clone)]
pub enum AlterRoleAction {
    /// Add a new permission grant to the role
    AddPermission(PermissionGrant),
    /// Drop a permission by index
    DropPermission(usize),
    /// Add parent roles to inherit from
    AddInherits(Vec<String>),
    /// Remove parent roles from inheritance
    DropInherits(Vec<String>),
    /// Update the role description
    SetDescription(String),
}

/// DROP ROLE statement
#[derive(Debug, Clone)]
pub struct DropRole {
    /// Role to drop
    pub role_id: String,
    /// If true, do not error when the role does not exist
    pub if_exists: bool,
}

/// SHOW ROLES statement
#[derive(Debug, Clone)]
pub struct ShowRoles {
    /// Optional LIKE pattern to filter roles
    pub like_pattern: Option<String>,
}

/// DESCRIBE ROLE statement
#[derive(Debug, Clone)]
pub struct DescribeRole {
    /// Role to describe
    pub role_id: String,
}

// ---------------------------------------------------------------------------
// Group statements
// ---------------------------------------------------------------------------

/// CREATE GROUP statement
///
/// Defines a new group with optional roles.
#[derive(Debug, Clone)]
pub struct CreateGroup {
    /// Unique group identifier
    pub group_id: String,
    /// Optional human-readable description
    pub description: Option<String>,
    /// Roles assigned to this group
    pub roles: Vec<String>,
}

/// ALTER GROUP statement
///
/// Modifies an existing group's roles or description.
#[derive(Debug, Clone)]
pub struct AlterGroup {
    /// Group to alter
    pub group_id: String,
    /// The alteration action to perform
    pub action: AlterGroupAction,
}

/// Actions that can be performed when altering a group
#[derive(Debug, Clone)]
pub enum AlterGroupAction {
    /// Add roles to the group
    AddRoles(Vec<String>),
    /// Remove roles from the group
    DropRoles(Vec<String>),
    /// Update the group description
    SetDescription(String),
}

/// DROP GROUP statement
#[derive(Debug, Clone)]
pub struct DropGroup {
    /// Group to drop
    pub group_id: String,
    /// If true, do not error when the group does not exist
    pub if_exists: bool,
}

/// SHOW GROUPS statement
#[derive(Debug, Clone)]
pub struct ShowGroups {
    /// Optional LIKE pattern to filter groups
    pub like_pattern: Option<String>,
}

/// DESCRIBE GROUP statement
#[derive(Debug, Clone)]
pub struct DescribeGroup {
    /// Group to describe
    pub group_id: String,
}

// ---------------------------------------------------------------------------
// User statements
// ---------------------------------------------------------------------------

/// CREATE USER statement
///
/// Defines a new user with authentication details and role/group assignments.
#[derive(Debug, Clone)]
pub struct CreateUser {
    /// Unique user identifier
    pub user_id: String,
    /// User email address
    pub email: String,
    /// Optional display name
    pub display_name: Option<String>,
    /// Roles directly assigned to this user
    pub roles: Vec<String>,
    /// Groups this user belongs to
    pub groups: Vec<String>,
    /// Whether the user can log in
    pub can_login: Option<bool>,
    /// Optional birth date (ISO 8601 string)
    pub birth_date: Option<String>,
    /// Optional folder path (IN FOLDER clause)
    pub folder: Option<String>,
}

/// ALTER USER statement
///
/// Modifies an existing user's properties or assignments.
#[derive(Debug, Clone)]
pub struct AlterUser {
    /// User to alter
    pub user_id: String,
    /// The alteration action to perform
    pub action: AlterUserAction,
}

/// Actions that can be performed when altering a user
#[derive(Debug, Clone)]
pub enum AlterUserAction {
    /// Add roles to the user
    AddRoles(Vec<String>),
    /// Remove roles from the user
    DropRoles(Vec<String>),
    /// Add the user to groups
    AddGroups(Vec<String>),
    /// Remove the user from groups
    DropGroups(Vec<String>),
    /// Update the user email
    SetEmail(String),
    /// Update the user display name
    SetDisplayName(String),
    /// Enable or disable login
    SetCanLogin(bool),
    /// Update the user birth date
    SetBirthDate(String),
}

/// DROP USER statement
#[derive(Debug, Clone)]
pub struct DropUser {
    /// User to drop
    pub user_id: String,
    /// If true, do not error when the user does not exist
    pub if_exists: bool,
}

/// SHOW USERS statement
#[derive(Debug, Clone)]
pub struct ShowUsers {
    /// Optional LIKE pattern to filter users
    pub like_pattern: Option<String>,
    /// Optional filter: only show users in this group
    pub in_group: Option<String>,
    /// Optional filter: only show users with this role
    pub with_role: Option<String>,
}

/// DESCRIBE USER statement
#[derive(Debug, Clone)]
pub struct DescribeUser {
    /// User to describe
    pub user_id: String,
}

// ---------------------------------------------------------------------------
// Grant / Revoke statements
// ---------------------------------------------------------------------------

/// Target of a GRANT statement
#[derive(Debug, Clone)]
pub enum GrantTarget {
    /// Grant to a specific user
    User(String),
    /// Grant to a group
    Group(String),
}

/// An item being granted
#[derive(Debug, Clone)]
pub enum GrantItem {
    /// Grant a role
    Role(String),
    /// Grant membership in a group
    Group(String),
}

/// GRANT statement
///
/// Assigns roles or group memberships to a user or group.
#[derive(Debug, Clone)]
pub struct Grant {
    /// The target receiving the grants
    pub target: GrantTarget,
    /// Items being granted
    pub grants: Vec<GrantItem>,
}

/// Target of a REVOKE statement
#[derive(Debug, Clone)]
pub enum RevokeTarget {
    /// Revoke from a specific user
    User(String),
    /// Revoke from a group
    Group(String),
}

/// An item being revoked
#[derive(Debug, Clone)]
pub enum RevokeItem {
    /// Revoke a role
    Role(String),
    /// Revoke membership in a group
    Group(String),
}

/// REVOKE statement
///
/// Removes roles or group memberships from a user or group.
#[derive(Debug, Clone)]
pub struct Revoke {
    /// The target losing the revocations
    pub target: RevokeTarget,
    /// Items being revoked
    pub revocations: Vec<RevokeItem>,
}

// ---------------------------------------------------------------------------
// Security configuration statements
// ---------------------------------------------------------------------------

/// A single security configuration setting
#[derive(Debug, Clone)]
pub enum SecurityConfigSetting {
    /// Set the default access policy (e.g., "deny-all", "allow-authenticated")
    DefaultPolicy(String),
    /// Enable or disable anonymous access
    AnonymousEnabled(bool),
    /// Set the role used for anonymous access
    AnonymousRole(String),
    /// Set an interface-specific setting (e.g., pgwire.require_tls = true)
    InterfaceSetting {
        /// Interface name (e.g., "pgwire", "http", "ws")
        interface: String,
        /// Setting key
        key: String,
        /// Setting value
        value: String,
    },
}

/// ALTER SECURITY CONFIG statement
///
/// Modifies security settings for a workspace pattern.
#[derive(Debug, Clone)]
pub struct AlterSecurityConfig {
    /// Workspace glob pattern the settings apply to
    pub workspace_pattern: String,
    /// Settings to apply
    pub settings: Vec<SecurityConfigSetting>,
}

/// SHOW SECURITY CONFIG statement
#[derive(Debug, Clone)]
pub struct ShowSecurityConfig {
    /// Optional workspace to show config for (shows all if None)
    pub workspace: Option<String>,
}

// ---------------------------------------------------------------------------
// Introspection statements
// ---------------------------------------------------------------------------

/// SHOW PERMISSIONS FOR statement
///
/// Displays the resolved permissions for a given user, optionally scoped to a workspace.
#[derive(Debug, Clone)]
pub struct ShowPermissionsFor {
    /// User whose permissions to display
    pub user_id: String,
    /// Optional workspace scope
    pub workspace: Option<String>,
}

/// SHOW EFFECTIVE ROLES FOR statement
///
/// Displays all roles (direct and inherited) for a given user.
#[derive(Debug, Clone)]
pub struct ShowEffectiveRolesFor {
    /// User whose effective roles to display
    pub user_id: String,
}
