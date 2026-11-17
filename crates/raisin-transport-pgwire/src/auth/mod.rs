// SPDX-License-Identifier: BSL-1.1

//! Authentication handler for PostgreSQL wire protocol.
//!
//! Implements API key-based authentication for pgwire connections.
//! Clients connect using: `postgresql://{tenant_id}:{api_token}@{host}:5432/{repository}`

mod context;
mod handler;
mod validator;

// Re-export public API
pub use context::ConnectionContext;
pub use handler::RaisinAuthHandler;
pub use validator::{ApiKeyValidator, ValidatedApiKey};

#[cfg(test)]
pub use validator::MockApiKeyValidator;

#[cfg(test)]
mod tests {
    use super::*;
    use pgwire::api::auth::DefaultServerParameterProvider;

    #[test]
    fn test_connection_context_creation() {
        let ctx = ConnectionContext::new(
            "tenant1".to_string(),
            "user123".to_string(),
            "repo1".to_string(),
        );

        assert_eq!(ctx.tenant_id, "tenant1");
        assert_eq!(ctx.user_id, "user123");
        assert_eq!(ctx.repository, "repo1");
    }

    #[test]
    fn test_mock_validator() {
        let mut validator = MockApiKeyValidator::new();
        validator.add_key(
            "raisin_test123".to_string(),
            "user1".to_string(),
            "tenant1".to_string(),
        );
        validator.grant_pgwire_access("tenant1".to_string(), "user1".to_string());

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let result = runtime.block_on(validator.validate_api_key("raisin_test123"));
        assert!(result.is_ok());
        assert_eq!(
            result.unwrap(),
            Some(("user1".to_string(), "tenant1".to_string()))
        );

        let result = runtime.block_on(validator.validate_api_key("invalid_key"));
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), None);

        let result = runtime.block_on(validator.has_pgwire_access("tenant1", "user1"));
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), true);

        let result = runtime.block_on(validator.has_pgwire_access("tenant1", "user2"));
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), false);
    }

    /// A scoped key's context is what its queries run with, unless `SET
    /// app.user` set an identity; `RESET app.user` falls back to the KEY's
    /// context, never to the system.
    #[test]
    fn a_scoped_connection_runs_queries_as_the_key() {
        use raisin_models::auth::AuthContext;
        let key = AuthContext::for_user("apikey:k1");
        let mut ctx = ConnectionContext::new("t".into(), "__scoped__".into(), "studio".into())
            .with_key_auth(key);
        assert!(ctx.is_scoped());
        let run_as = ctx.effective_auth().expect("a context");
        assert_eq!(run_as.user_id.as_deref(), Some("apikey:k1"));
        assert!(!run_as.is_system);

        ctx.set_identity_auth(AuthContext::for_user("someone"));
        assert_eq!(
            ctx.effective_auth().unwrap().user_id.as_deref(),
            Some("someone")
        );
        ctx.clear_identity_auth();
        assert_eq!(
            ctx.effective_auth().unwrap().user_id.as_deref(),
            Some("apikey:k1")
        );

        // An admin key's connection is unchanged: no context, queries as before.
        let admin = ConnectionContext::new("t".into(), "admin".into(), "studio".into());
        assert!(!admin.is_scoped());
        assert!(admin.effective_auth().is_none());
    }

    /// A validator that knows nothing of scopes treats every key as unscoped.
    #[test]
    fn the_default_scoped_validation_is_unscoped() {
        let mut validator = MockApiKeyValidator::new();
        validator.add_key("raisin_k".into(), "u".into(), "t".into());
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let v = runtime
            .block_on(validator.validate_api_key_scoped("raisin_k"))
            .unwrap()
            .unwrap();
        assert_eq!((v.user_id.as_str(), v.tenant_id.as_str()), ("u", "t"));
        assert!(v.scope_repo.is_none() && v.key_auth.is_none());
    }

    #[test]
    fn test_auth_handler_creation() {
        let validator = MockApiKeyValidator::new();
        let params = DefaultServerParameterProvider::default();
        let _handler = RaisinAuthHandler::new(validator, params);
    }
}
