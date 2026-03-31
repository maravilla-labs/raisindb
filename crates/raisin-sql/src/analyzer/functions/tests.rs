use super::*;
use crate::analyzer::types::DataType;

#[test]
fn test_resolve_exact_match() {
    let registry = FunctionRegistry::default();

    // DEPTH(PATH) -> INT
    let sig = registry
        .resolve("DEPTH", &[DataType::Path])
        .expect("DEPTH exists");
    assert_eq!(sig.return_type, DataType::Int);
    assert_eq!(sig.category, FunctionCategory::Hierarchy);

    // PARENT(PATH) -> PATH?
    let sig = registry
        .resolve("PARENT", &[DataType::Path])
        .expect("PARENT exists");
    assert_eq!(
        sig.return_type,
        DataType::Nullable(Box::new(DataType::Path))
    );
}

#[test]
fn test_resolve_with_coercion() {
    let registry = FunctionRegistry::default();

    // TEXT can coerce to PATH
    let sig = registry
        .resolve("DEPTH", &[DataType::Text])
        .expect("DEPTH with TEXT coerces to PATH");
    assert_eq!(sig.return_type, DataType::Int);
}

#[test]
fn test_resolve_json_functions() {
    let registry = FunctionRegistry::default();

    let sig = registry
        .resolve("JSON_VALUE", &[DataType::JsonB, DataType::Text])
        .expect("JSON_VALUE exists");
    assert_eq!(
        sig.return_type,
        DataType::Nullable(Box::new(DataType::Text))
    );

    let sig = registry
        .resolve("JSON_GET_DOUBLE", &[DataType::JsonB, DataType::Text])
        .expect("JSON_GET_DOUBLE exists");
    assert_eq!(
        sig.return_type,
        DataType::Nullable(Box::new(DataType::Double))
    );
}

#[test]
fn test_resolve_aggregate_functions() {
    let registry = FunctionRegistry::default();

    // COUNT() with no args
    let sig = registry.resolve("COUNT", &[]).expect("COUNT() exists");
    assert_eq!(sig.return_type, DataType::BigInt);
    assert_eq!(sig.category, FunctionCategory::Aggregate);

    // COUNT(*) / COUNT(column)
    let sig = registry
        .resolve("COUNT", &[DataType::Unknown])
        .expect("COUNT(*) exists");
    assert_eq!(sig.return_type, DataType::BigInt);
}

#[test]
fn test_resolve_scalar_functions() {
    let registry = FunctionRegistry::default();

    let sig = registry
        .resolve("LOWER", &[DataType::Text])
        .expect("LOWER exists");
    assert_eq!(sig.return_type, DataType::Text);

    let sig = registry
        .resolve("LENGTH", &[DataType::Text])
        .expect("LENGTH exists");
    assert_eq!(sig.return_type, DataType::Int);
}

#[test]
fn test_resolve_case_insensitive() {
    let registry = FunctionRegistry::default();

    assert!(registry.resolve("depth", &[DataType::Path]).is_some());
    assert!(registry.resolve("DEPTH", &[DataType::Path]).is_some());
    assert!(registry.resolve("Depth", &[DataType::Path]).is_some());
}

#[test]
fn test_resolve_wrong_arg_count() {
    let registry = FunctionRegistry::default();

    // DEPTH takes 1 argument
    assert!(registry.resolve("DEPTH", &[]).is_none());
    assert!(registry
        .resolve("DEPTH", &[DataType::Path, DataType::Path])
        .is_none());
}

#[test]
fn test_resolve_wrong_arg_type() {
    let registry = FunctionRegistry::default();

    // DEPTH requires PATH (or coercible to PATH)
    assert!(registry.resolve("DEPTH", &[DataType::Int]).is_none());
    assert!(registry.resolve("DEPTH", &[DataType::JsonB]).is_none());
}

#[test]
fn test_get_signatures() {
    let registry = FunctionRegistry::default();

    let sigs = registry
        .get_signatures("COUNT")
        .expect("COUNT has signatures");
    assert!(sigs.len() >= 2); // COUNT() and COUNT(*)

    let sigs = registry
        .get_signatures("DEPTH")
        .expect("DEPTH has signatures");
    assert_eq!(sigs.len(), 1);
}

#[test]
fn test_function_not_found() {
    let registry = FunctionRegistry::default();
    assert!(registry
        .resolve("NONEXISTENT_FUNCTION", &[DataType::Int])
        .is_none());
}

#[test]
fn test_path_starts_with() {
    let registry = FunctionRegistry::default();

    let sig = registry
        .resolve("PATH_STARTS_WITH", &[DataType::Path, DataType::Path])
        .expect("PATH_STARTS_WITH exists");
    assert_eq!(sig.return_type, DataType::Boolean);

    // With TEXT coercion
    let sig = registry
        .resolve("PATH_STARTS_WITH", &[DataType::Path, DataType::Text])
        .expect("PATH_STARTS_WITH with TEXT coercion");
    assert_eq!(sig.return_type, DataType::Boolean);
}

// ========================================================================
// Auth Function Tests
// ========================================================================

#[test]
fn test_resolve_auth_current_user() {
    let registry = FunctionRegistry::default();

    let sig = registry
        .resolve("RAISIN_AUTH_CURRENT_USER", &[])
        .expect("RAISIN_AUTH_CURRENT_USER exists");
    assert_eq!(
        sig.return_type,
        DataType::Nullable(Box::new(DataType::Text))
    );
    assert_eq!(sig.category, FunctionCategory::Auth);
    assert!(!sig.is_deterministic);
}

#[test]
fn test_resolve_auth_has_permission() {
    let registry = FunctionRegistry::default();

    let sig = registry
        .resolve(
            "RAISIN_AUTH_HAS_PERMISSION",
            &[DataType::Text, DataType::Text],
        )
        .expect("RAISIN_AUTH_HAS_PERMISSION exists");
    assert_eq!(sig.return_type, DataType::Boolean);
    assert_eq!(sig.category, FunctionCategory::Auth);
}

#[test]
fn test_resolve_auth_add_provider() {
    let registry = FunctionRegistry::default();

    let sig = registry
        .resolve(
            "RAISIN_AUTH_ADD_PROVIDER",
            &[DataType::Text, DataType::Text],
        )
        .expect("RAISIN_AUTH_ADD_PROVIDER exists");
    assert_eq!(sig.return_type, DataType::Text);
    assert_eq!(sig.category, FunctionCategory::Auth);
    assert!(!sig.is_deterministic); // Mutation function
}

#[test]
fn test_resolve_auth_get_settings() {
    let registry = FunctionRegistry::default();

    let sig = registry
        .resolve("RAISIN_AUTH_GET_SETTINGS", &[])
        .expect("RAISIN_AUTH_GET_SETTINGS exists");
    assert_eq!(sig.return_type, DataType::JsonB);
    assert_eq!(sig.category, FunctionCategory::Auth);
}

#[test]
fn test_auth_function_case_insensitive() {
    let registry = FunctionRegistry::default();

    // All should resolve to the same function
    assert!(registry.resolve("raisin_auth_current_user", &[]).is_some());
    assert!(registry.resolve("RAISIN_AUTH_CURRENT_USER", &[]).is_some());
    assert!(registry.resolve("Raisin_Auth_Current_User", &[]).is_some());
}
