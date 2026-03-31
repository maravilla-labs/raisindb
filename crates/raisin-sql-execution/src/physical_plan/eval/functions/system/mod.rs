//! System information functions
//!
//! This module contains PostgreSQL-compatible system information functions:
//! - VERSION: Return database version string
//! - CURRENT_SCHEMA: Return current schema name
//! - CURRENT_DATABASE: Return current database name
//! - CURRENT_USER: Return current user node from repository (as JSON)
//!
//! These functions are used by PostgreSQL clients (pgAdmin, DBeaver, etc.)
//! during connection initialization to discover server capabilities.
//!
//! The CURRENT_USER function uses thread-local context to access the
//! authenticated user's node set by the query engine.

use crate::physical_plan::eval::functions::get_function_context;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::registry::FunctionRegistry;

/// Return database version string
///
/// # SQL Signature
/// `VERSION() -> TEXT`
///
/// # Examples
/// ```sql
/// SELECT VERSION() -> 'RaisinDB 0.1.0'
/// ```
pub struct VersionFunction;

impl SqlFunction for VersionFunction {
    fn name(&self) -> &str {
        "VERSION"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::System
    }

    fn signature(&self) -> &str {
        "VERSION() -> TEXT"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], _row: &Row) -> Result<Literal, Error> {
        if !args.is_empty() {
            return Err(Error::Validation(
                "VERSION requires exactly 0 arguments".to_string(),
            ));
        }
        // Return RaisinDB version with PostgreSQL-compatible format
        Ok(Literal::Text(format!(
            "RaisinDB {} on {} (PostgreSQL-compatible)",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS
        )))
    }
}

/// Return current schema name
///
/// # SQL Signature
/// `CURRENT_SCHEMA() -> TEXT`
///
/// # Examples
/// ```sql
/// SELECT CURRENT_SCHEMA() -> 'public'
/// ```
pub struct CurrentSchemaFunction;

impl SqlFunction for CurrentSchemaFunction {
    fn name(&self) -> &str {
        "CURRENT_SCHEMA"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::System
    }

    fn signature(&self) -> &str {
        "CURRENT_SCHEMA() -> TEXT"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], _row: &Row) -> Result<Literal, Error> {
        if !args.is_empty() {
            return Err(Error::Validation(
                "CURRENT_SCHEMA requires exactly 0 arguments".to_string(),
            ));
        }
        // RaisinDB uses "public" as the default schema (PostgreSQL convention)
        Ok(Literal::Text("public".to_string()))
    }
}

/// Return current database name
///
/// # SQL Signature
/// `CURRENT_DATABASE() -> TEXT`
///
/// # Examples
/// ```sql
/// SELECT CURRENT_DATABASE() -> 'raisindb'
/// ```
pub struct CurrentDatabaseFunction;

impl SqlFunction for CurrentDatabaseFunction {
    fn name(&self) -> &str {
        "CURRENT_DATABASE"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::System
    }

    fn signature(&self) -> &str {
        "CURRENT_DATABASE() -> TEXT"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], _row: &Row) -> Result<Literal, Error> {
        if !args.is_empty() {
            return Err(Error::Validation(
                "CURRENT_DATABASE requires exactly 0 arguments".to_string(),
            ));
        }
        // Return a generic database name (the actual repo is in the connection context)
        Ok(Literal::Text("raisindb".to_string()))
    }
}

/// Return current user node from the repository
///
/// # SQL Signature
/// `RAISIN_CURRENT_USER() -> JSON`
///
/// # Returns
/// The authenticated user's node from the access_control workspace as JSON
/// Returns NULL if not authenticated or user node not available
///
/// # Examples
/// ```sql
/// SELECT RAISIN_CURRENT_USER();  -- Returns user node with path, properties, etc.
/// SELECT RAISIN_CURRENT_USER()->>'path';  -- Returns '/users/internal/john-at-example-com'
/// ```
///
/// # Note
/// This function is named `RAISIN_CURRENT_USER` (not `CURRENT_USER`) because
/// `CURRENT_USER` is a SQL reserved keyword that conflicts with the parser.
pub struct RaisinCurrentUserFunction;

impl SqlFunction for RaisinCurrentUserFunction {
    fn name(&self) -> &str {
        "RAISIN_CURRENT_USER"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::System
    }

    fn signature(&self) -> &str {
        "RAISIN_CURRENT_USER() -> JSON"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], _row: &Row) -> Result<Literal, Error> {
        if !args.is_empty() {
            return Err(Error::Validation(
                "RAISIN_CURRENT_USER requires exactly 0 arguments".to_string(),
            ));
        }
        // Get user_node from thread-local function context
        match get_function_context() {
            Some(ctx) => match ctx.user_node {
                Some(node) => Ok(Literal::JsonB(node)),
                None => Ok(Literal::Null), // No user node available
            },
            None => Ok(Literal::Null), // No auth context available
        }
    }
}

/// Return session user name (PostgreSQL compatibility)
///
/// # SQL Signature
/// `SESSION_USER -> TEXT`
pub struct SessionUserFunction;

impl SqlFunction for SessionUserFunction {
    fn name(&self) -> &str {
        "SESSION_USER"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::System
    }

    fn signature(&self) -> &str {
        "SESSION_USER -> TEXT"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], _row: &Row) -> Result<Literal, Error> {
        if !args.is_empty() {
            return Err(Error::Validation(
                "SESSION_USER requires exactly 0 arguments".to_string(),
            ));
        }
        Ok(Literal::Text("raisindb".to_string()))
    }
}

/// Return current catalog name (PostgreSQL compatibility - same as database)
///
/// # SQL Signature
/// `CURRENT_CATALOG -> TEXT`
pub struct CurrentCatalogFunction;

impl SqlFunction for CurrentCatalogFunction {
    fn name(&self) -> &str {
        "CURRENT_CATALOG"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::System
    }

    fn signature(&self) -> &str {
        "CURRENT_CATALOG -> TEXT"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], _row: &Row) -> Result<Literal, Error> {
        if !args.is_empty() {
            return Err(Error::Validation(
                "CURRENT_CATALOG requires exactly 0 arguments".to_string(),
            ));
        }
        Ok(Literal::Text("raisindb".to_string()))
    }
}

/// Register all system functions in the provided registry
pub fn register_functions(registry: &mut FunctionRegistry) {
    registry.register(Box::new(VersionFunction));
    registry.register(Box::new(CurrentSchemaFunction));
    registry.register(Box::new(CurrentDatabaseFunction));
    registry.register(Box::new(RaisinCurrentUserFunction));
    registry.register(Box::new(SessionUserFunction));
    registry.register(Box::new(CurrentCatalogFunction));
}
