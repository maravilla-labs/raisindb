// SPDX-License-Identifier: BSL-1.1

//! Connection context stored after successful authentication.

use raisin_models::auth::AuthContext;

/// Connection context created after successful authentication
#[derive(Debug, Clone)]
pub struct ConnectionContext {
    /// Tenant ID extracted from the username field
    pub tenant_id: String,
    /// User ID from the validated API key (admin user)
    pub user_id: String,
    /// Repository name extracted from the database field
    pub repository: String,
    /// Identity user auth context (set via SET app.user = '<jwt>')
    pub identity_auth: Option<AuthContext>,
    /// Session branch (set via USE BRANCH or SET app.branch)
    pub session_branch: Option<String>,
    /// The RLS context of a SCOPED API key (its roles in this repository).
    /// `None` for an admin's own key, whose queries run as before.
    pub key_auth: Option<AuthContext>,
}

impl ConnectionContext {
    /// Create a new connection context
    pub fn new(tenant_id: String, user_id: String, repository: String) -> Self {
        Self {
            tenant_id,
            user_id,
            repository,
            identity_auth: None,
            session_branch: None,
            key_auth: None,
        }
    }

    /// The same context for a scoped key.
    pub fn with_key_auth(mut self, auth: AuthContext) -> Self {
        self.key_auth = Some(auth);
        self
    }

    /// Whether this connection authenticated with a scoped key.
    pub fn is_scoped(&self) -> bool {
        self.key_auth.is_some()
    }

    /// The context a query runs with: the identity set by `SET app.user`,
    /// else a scoped key's own context, else none (an admin key: unchanged).
    pub fn effective_auth(&self) -> Option<AuthContext> {
        self.identity_auth.clone().or_else(|| self.key_auth.clone())
    }

    /// Set identity auth context (from SET app.user = '<jwt>')
    pub fn set_identity_auth(&mut self, auth: AuthContext) {
        self.identity_auth = Some(auth);
    }

    /// Clear identity auth context (from RESET app.user)
    pub fn clear_identity_auth(&mut self) {
        self.identity_auth = None;
    }

    /// Get identity auth context if set
    pub fn identity_auth(&self) -> Option<&AuthContext> {
        self.identity_auth.as_ref()
    }

    /// Set session branch (from USE BRANCH or SET app.branch)
    pub fn set_session_branch(&mut self, branch: String) {
        self.session_branch = Some(branch);
    }

    /// Clear session branch (returns to default)
    pub fn clear_session_branch(&mut self) {
        self.session_branch = None;
    }

    /// Get session branch if set
    pub fn session_branch(&self) -> Option<&str> {
        self.session_branch.as_deref()
    }
}
