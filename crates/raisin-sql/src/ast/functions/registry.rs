//! RaisinDB function registry
//!
//! Defines the `RaisinFunction` enum and its associated methods for
//! looking up functions by name, checking arity, and distinguishing
//! table-valued from scalar functions.

/// Supported RaisinDB-specific functions
#[derive(Debug, Clone, PartialEq)]
pub enum RaisinFunction {
    /// PATH_STARTS_WITH(path, prefix) - Hierarchy function
    PathStartsWith,
    /// PARENT(path) - Returns parent path
    Parent,
    /// DEPTH(path) - Returns path depth
    Depth,
    /// JSON_VALUE(properties, jsonpath) - Extract JSON value
    JsonValue,
    /// JSON_EXISTS(properties, jsonpath) - Check JSON key existence
    JsonExists,
    /// KNN(query_vector, k, filter) - K-nearest neighbors (table-valued)
    Knn,
    /// NEIGHBORS(start_id, direction, label) - Graph traversal (table-valued)
    Neighbors,
    /// CYPHER(query_string) - Execute Cypher graph query (table-valued) [DEPRECATED]
    Cypher,
    /// GRAPH_TABLE(pattern) - SQL/PGQ property graph query (table-valued)
    GraphTable,

    // ========================================================================
    // Authentication Configuration Functions (table-valued)
    // ========================================================================
    /// RAISIN_AUTH_PROVIDERS() - List configured auth providers (table-valued)
    /// Returns: provider_id, strategy_type, display_name, icon, enabled, created_at
    RaisinAuthProviders,
    /// RAISIN_AUTH_IDENTITIES(filter?) - List identities (table-valued, admin only)
    /// Returns: identity_id, email, display_name, email_verified, is_active, created_at, last_login_at
    RaisinAuthIdentities,
    /// RAISIN_AUTH_SESSIONS(identity_id?) - List sessions (table-valued, admin only)
    /// Returns: session_id, identity_id, auth_strategy, user_agent, ip_address, created_at, last_active_at
    RaisinAuthSessions,
    /// RAISIN_AUTH_ACCESS_REQUESTS(repo_id?, status?) - List workspace access requests (table-valued)
    /// Returns: request_id, identity_id, email, repo_id, status, message, created_at
    RaisinAuthAccessRequests,

    // ========================================================================
    // Authentication Configuration Functions (scalar - mutations)
    // ========================================================================
    /// RAISIN_AUTH_ADD_PROVIDER(strategy_type, config_json) - Add auth provider
    /// Example: SELECT RAISIN_AUTH_ADD_PROVIDER('oidc:google', '{"client_id": "...", "client_secret": "..."}')
    RaisinAuthAddProvider,
    /// RAISIN_AUTH_UPDATE_PROVIDER(provider_id, config_json) - Update auth provider
    RaisinAuthUpdateProvider,
    /// RAISIN_AUTH_REMOVE_PROVIDER(provider_id) - Remove auth provider
    RaisinAuthRemoveProvider,
    /// RAISIN_AUTH_GET_SETTINGS() - Get current auth settings as JSON
    RaisinAuthGetSettings,
    /// RAISIN_AUTH_UPDATE_SETTINGS(settings_json) - Update auth settings
    /// Settings include: session_duration_hours, password_policy, access_settings
    RaisinAuthUpdateSettings,
    /// RAISIN_AUTH_CURRENT_USER() - Get current authenticated user's identity_id
    RaisinAuthCurrentUser,
    /// RAISIN_AUTH_CURRENT_WORKSPACE() - Get current workspace context
    RaisinAuthCurrentWorkspace,
    /// RAISIN_AUTH_HAS_PERMISSION(resource, permission) - Check if current user has permission
    RaisinAuthHasPermission,
}

impl RaisinFunction {
    /// Get the inclusive argument bounds (min, max)
    pub fn arity_bounds(&self) -> (usize, usize) {
        match self {
            Self::Cypher => (1, 2),
            Self::PathStartsWith => (2, 2),
            Self::Parent => (1, 1),
            Self::Depth => (1, 1),
            Self::JsonValue => (2, 2),
            Self::JsonExists => (2, 2),
            Self::Knn => (3, 3),
            Self::Neighbors => (3, 3),
            Self::GraphTable => (1, 1), // Takes the raw GRAPH_TABLE expression as a string
            // Auth table-valued functions
            Self::RaisinAuthProviders => (0, 0),  // No arguments
            Self::RaisinAuthIdentities => (0, 1), // Optional filter JSON
            Self::RaisinAuthSessions => (0, 1),   // Optional identity_id filter
            Self::RaisinAuthAccessRequests => (0, 2), // Optional repo_id, status
            // Auth scalar functions
            Self::RaisinAuthAddProvider => (2, 2), // strategy_type, config_json
            Self::RaisinAuthUpdateProvider => (2, 2), // provider_id, config_json
            Self::RaisinAuthRemoveProvider => (1, 1), // provider_id
            Self::RaisinAuthGetSettings => (0, 0), // No arguments
            Self::RaisinAuthUpdateSettings => (1, 1), // settings_json
            Self::RaisinAuthCurrentUser => (0, 0), // No arguments
            Self::RaisinAuthCurrentWorkspace => (0, 0), // No arguments
            Self::RaisinAuthHasPermission => (2, 2), // resource, permission
        }
    }

    /// Human readable description of allowed argument counts
    pub fn arity_description(&self) -> String {
        let (min, max) = self.arity_bounds();
        if min == max {
            format!("{}", min)
        } else if max == min + 1 {
            format!("{} or {}", min, max)
        } else {
            format!("between {} and {}", min, max)
        }
    }

    /// Check if the provided argument count is valid for this function
    pub fn allows_arg_count(&self, count: usize) -> bool {
        let (min, max) = self.arity_bounds();
        count >= min && count <= max
    }

    /// Parse function name into RaisinFunction enum
    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_uppercase().as_str() {
            "PATH_STARTS_WITH" => Some(Self::PathStartsWith),
            "PARENT" => Some(Self::Parent),
            "DEPTH" => Some(Self::Depth),
            "JSON_VALUE" => Some(Self::JsonValue),
            "JSON_EXISTS" => Some(Self::JsonExists),
            "KNN" => Some(Self::Knn),
            "NEIGHBORS" => Some(Self::Neighbors),
            "CYPHER" => Some(Self::Cypher),
            "GRAPH_TABLE" => Some(Self::GraphTable),
            // Auth table-valued functions
            "RAISIN_AUTH_PROVIDERS" => Some(Self::RaisinAuthProviders),
            "RAISIN_AUTH_IDENTITIES" => Some(Self::RaisinAuthIdentities),
            "RAISIN_AUTH_SESSIONS" => Some(Self::RaisinAuthSessions),
            "RAISIN_AUTH_ACCESS_REQUESTS" => Some(Self::RaisinAuthAccessRequests),
            // Auth scalar functions
            "RAISIN_AUTH_ADD_PROVIDER" => Some(Self::RaisinAuthAddProvider),
            "RAISIN_AUTH_UPDATE_PROVIDER" => Some(Self::RaisinAuthUpdateProvider),
            "RAISIN_AUTH_REMOVE_PROVIDER" => Some(Self::RaisinAuthRemoveProvider),
            "RAISIN_AUTH_GET_SETTINGS" => Some(Self::RaisinAuthGetSettings),
            "RAISIN_AUTH_UPDATE_SETTINGS" => Some(Self::RaisinAuthUpdateSettings),
            "RAISIN_AUTH_CURRENT_USER" => Some(Self::RaisinAuthCurrentUser),
            "RAISIN_AUTH_CURRENT_WORKSPACE" => Some(Self::RaisinAuthCurrentWorkspace),
            "RAISIN_AUTH_HAS_PERMISSION" => Some(Self::RaisinAuthHasPermission),
            _ => None,
        }
    }

    /// Check if this is a table-valued function
    pub fn is_table_valued(&self) -> bool {
        matches!(
            self,
            Self::Knn
                | Self::Neighbors
                | Self::Cypher
                | Self::GraphTable
                | Self::RaisinAuthProviders
                | Self::RaisinAuthIdentities
                | Self::RaisinAuthSessions
                | Self::RaisinAuthAccessRequests
        )
    }
}
