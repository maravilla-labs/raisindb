//! RaisinDB-specific function validation
//!
//! This module validates custom functions used in RaisinSQL:
//! - Hierarchy: PATH_STARTS_WITH, PARENT, DEPTH
//! - JSON: JSON_VALUE, JSON_EXISTS (in addition to Postgres operators)
//! - Vector: KNN (table-valued function)
//! - Graph: NEIGHBORS (table-valued function)
//! - Auth: RAISIN_AUTH_* functions for authentication configuration

mod registry;
mod validation;

pub use registry::RaisinFunction;
pub use validation::validate_raisin_functions;
pub(crate) use validation::validate_table_names_in_query;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::parser::parse_sql;

    #[test]
    fn test_path_starts_with_valid() {
        let sql = "SELECT * FROM nodes WHERE PATH_STARTS_WITH(path, '/content/')";
        let result = parse_sql(sql);
        assert!(result.is_ok());
    }

    #[test]
    fn test_path_starts_with_invalid_args() {
        let sql = "SELECT * FROM nodes WHERE PATH_STARTS_WITH(path)";
        let result = parse_sql(sql);
        assert!(result.is_err());
    }

    #[test]
    fn test_parent_function() {
        let sql = "SELECT * FROM nodes WHERE PARENT(path) = '/content'";
        let result = parse_sql(sql);
        assert!(result.is_ok());
    }

    #[test]
    fn test_depth_function() {
        let sql = "SELECT * FROM nodes WHERE DEPTH(path) > 2";
        let result = parse_sql(sql);
        assert!(result.is_ok());
    }

    #[test]
    fn test_json_value_function() {
        let sql = "SELECT JSON_VALUE(properties, '$.title') AS title FROM nodes";
        let result = parse_sql(sql);
        assert!(result.is_ok());
    }

    #[test]
    fn test_json_exists_function() {
        let sql = "SELECT * FROM nodes WHERE JSON_EXISTS(properties, '$.seo.title')";
        let result = parse_sql(sql);
        assert!(result.is_ok());
    }

    #[test]
    fn test_direct_json_column_select() {
        // In Postgres, you can select JSONB columns directly without TO_JSON
        let sql = "SELECT properties FROM nodes WHERE id = 'node-123'";
        let result = parse_sql(sql);
        assert!(result.is_ok());
    }

    #[test]
    fn test_multiple_functions() {
        let sql = r#"
            SELECT id, name, JSON_VALUE(properties, '$.title') AS title
            FROM nodes
            WHERE PATH_STARTS_WITH(path, '/content/')
            AND DEPTH(path) = 3
        "#;
        let result = parse_sql(sql);
        assert!(result.is_ok());
    }

    // ========================================================================
    // Auth Function Tests
    // ========================================================================

    #[test]
    fn test_raisin_auth_current_user() {
        // Scalar function in SELECT - should parse but may fail semantic validation
        let sql = "SELECT RAISIN_AUTH_CURRENT_USER() AS user_id FROM nodes WHERE id = 'dummy'";
        let result = parse_sql(sql);
        assert!(result.is_ok());
    }

    #[test]
    fn test_raisin_auth_has_permission() {
        let sql = "SELECT RAISIN_AUTH_HAS_PERMISSION('workspace:main', 'read') AS can_read FROM nodes WHERE id = 'dummy'";
        let result = parse_sql(sql);
        assert!(result.is_ok());
    }

    #[test]
    fn test_raisin_auth_has_permission_invalid_args() {
        // Should fail - needs 2 arguments
        let sql = "SELECT RAISIN_AUTH_HAS_PERMISSION('workspace:main') AS can_read FROM nodes WHERE id = 'dummy'";
        let result = parse_sql(sql);
        assert!(result.is_err());
    }

    #[test]
    fn test_raisin_auth_get_settings() {
        let sql = "SELECT RAISIN_AUTH_GET_SETTINGS() AS settings FROM nodes WHERE id = 'dummy'";
        let result = parse_sql(sql);
        assert!(result.is_ok());
    }

    #[test]
    fn test_raisin_auth_add_provider() {
        let sql = r#"SELECT RAISIN_AUTH_ADD_PROVIDER('oidc:google', '{"client_id": "xxx"}') AS provider_id FROM nodes WHERE id = 'dummy'"#;
        let result = parse_sql(sql);
        assert!(result.is_ok());
    }

    #[test]
    fn test_raisin_auth_update_settings() {
        let sql = r#"SELECT RAISIN_AUTH_UPDATE_SETTINGS('{"session_duration_hours": 48}') AS result FROM nodes WHERE id = 'dummy'"#;
        let result = parse_sql(sql);
        assert!(result.is_ok());
    }

    #[test]
    fn test_raisin_auth_function_enum() {
        // Test from_name for auth functions
        assert_eq!(
            RaisinFunction::from_name("RAISIN_AUTH_PROVIDERS"),
            Some(RaisinFunction::RaisinAuthProviders)
        );
        assert_eq!(
            RaisinFunction::from_name("raisin_auth_current_user"),
            Some(RaisinFunction::RaisinAuthCurrentUser)
        );
        assert_eq!(
            RaisinFunction::from_name("RAISIN_AUTH_HAS_PERMISSION"),
            Some(RaisinFunction::RaisinAuthHasPermission)
        );

        // Test is_table_valued
        assert!(RaisinFunction::RaisinAuthProviders.is_table_valued());
        assert!(RaisinFunction::RaisinAuthIdentities.is_table_valued());
        assert!(RaisinFunction::RaisinAuthSessions.is_table_valued());
        assert!(RaisinFunction::RaisinAuthAccessRequests.is_table_valued());

        // Scalar functions should NOT be table-valued
        assert!(!RaisinFunction::RaisinAuthCurrentUser.is_table_valued());
        assert!(!RaisinFunction::RaisinAuthHasPermission.is_table_valued());
        assert!(!RaisinFunction::RaisinAuthAddProvider.is_table_valued());
        assert!(!RaisinFunction::RaisinAuthGetSettings.is_table_valued());

        // Test arity bounds
        assert_eq!(RaisinFunction::RaisinAuthProviders.arity_bounds(), (0, 0));
        assert_eq!(RaisinFunction::RaisinAuthIdentities.arity_bounds(), (0, 1));
        assert_eq!(RaisinFunction::RaisinAuthAddProvider.arity_bounds(), (2, 2));
        assert_eq!(
            RaisinFunction::RaisinAuthHasPermission.arity_bounds(),
            (2, 2)
        );
    }
}
