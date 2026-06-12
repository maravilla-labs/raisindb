//! Node type-membership functions: HAS_MIXIN and IS_A.
//!
//! These are pure, fast membership tests over a node's materialized type sets.
//! On every write the engine stamps two reserved properties onto each node
//! (see `raisin_models::nodes`):
//! - `$mixins` — the node's effective mixin names (from its NodeType, transitively)
//! - `$supertypes` — its node_type, all `extends` ancestors, and every effective mixin
//!
//! Because the sets are materialized, these functions never resolve NodeTypes at
//! query time — they just test array membership on the row's `properties` object.

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_models::nodes::{RESERVED_MIXINS_KEY, RESERVED_SUPERTYPES_KEY};
use raisin_sql::analyzer::{Literal, TypedExpr};

/// True if `obj[key]` is a JSON string array containing `needle`.
fn json_string_array_contains(obj: &serde_json::Value, key: &str, needle: &str) -> bool {
    match obj.get(key) {
        Some(serde_json::Value::Array(items)) => items.iter().any(|v| v.as_str() == Some(needle)),
        _ => false,
    }
}

/// Shared evaluation for HAS_MIXIN / IS_A: `fn(properties, name) -> bool`.
fn eval_membership(
    fn_name: &str,
    key: &str,
    args: &[TypedExpr],
    row: &Row,
) -> Result<Literal, Error> {
    if args.len() != 2 {
        return Err(Error::Validation(format!(
            "{} requires exactly 2 arguments (properties, name)",
            fn_name
        )));
    }

    let json_val = eval_expr(&args[0], row)?;
    let name_val = eval_expr(&args[1], row)?;

    // A NULL/absent membership set means "no", not an error.
    if matches!(json_val, Literal::Null) || matches!(name_val, Literal::Null) {
        return Ok(Literal::Boolean(false));
    }

    let Literal::JsonB(obj) = json_val else {
        return Err(Error::Validation(format!(
            "{} first argument must be JSONB (the node's `properties`)",
            fn_name
        )));
    };
    let Literal::Text(name) = name_val else {
        return Err(Error::Validation(format!(
            "{} second argument must be TEXT",
            fn_name
        )));
    };

    Ok(Literal::Boolean(json_string_array_contains(
        &obj, key, &name,
    )))
}

/// `HAS_MIXIN(properties, mixin_name) -> BOOLEAN`
///
/// True when the node carries the given mixin (type-declared, transitively).
///
/// ```sql
/// SELECT * FROM 'workspace' WHERE HAS_MIXIN(properties, 'studio:SEO')
/// ```
pub struct HasMixinFunction;

impl SqlFunction for HasMixinFunction {
    fn name(&self) -> &str {
        "HAS_MIXIN"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::System
    }

    fn signature(&self) -> &str {
        "HAS_MIXIN(properties, mixin_name) -> BOOLEAN"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        eval_membership("HAS_MIXIN", RESERVED_MIXINS_KEY, args, row)
    }
}

/// `IS_A(properties, type_name) -> BOOLEAN`
///
/// True when the node "is a" given type — its node_type, any `extends` ancestor,
/// or any effective mixin. The `$supertypes` set
/// already includes the node_type, so the node_type need not be passed separately.
///
/// ```sql
/// SELECT * FROM 'workspace' WHERE IS_A(properties, 'studio:Folder')
/// ```
pub struct IsAFunction;

impl SqlFunction for IsAFunction {
    fn name(&self) -> &str {
        "IS_A"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::System
    }

    fn signature(&self) -> &str {
        "IS_A(properties, type_name) -> BOOLEAN"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        eval_membership("IS_A", RESERVED_SUPERTYPES_KEY, args, row)
    }
}

/// Register the node type-membership functions.
pub fn register_functions(registry: &mut super::registry::FunctionRegistry) {
    registry.register(Box::new(HasMixinFunction));
    registry.register(Box::new(IsAFunction));
}
