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
pub use validator::ApiKeyValidator;

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

    #[test]
    fn test_auth_handler_creation() {
        let validator = MockApiKeyValidator::new();
        let params = DefaultServerParameterProvider::default();
        let _handler = RaisinAuthHandler::new(validator, params);
    }
}
