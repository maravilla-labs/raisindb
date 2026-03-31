//! Tests for authentication service

use super::*;
use crate::RocksDBStorage;
use raisin_models::admin_user::AdminInterface;

fn create_test_service() -> (tempfile::TempDir, AuthService) {
    use crate::RocksDBConfig;

    let temp_dir = tempfile::TempDir::new().unwrap();
    let config = RocksDBConfig::default().with_path(temp_dir.path());
    let storage = RocksDBStorage::with_config(config).unwrap();

    let store = AdminUserStore::new(storage.db().clone());
    let service = AuthService::new(store, "test_secret_key".to_string());

    (temp_dir, service)
}

#[test]
fn test_password_hashing() {
    let password = "TestPassword123!";
    let hash = AuthService::hash_password(password).unwrap();

    assert!(AuthService::verify_password(password, &hash).unwrap());
    assert!(!AuthService::verify_password("wrong_password", &hash).unwrap());
}

#[test]
fn test_password_generation() {
    let password = AuthService::generate_password();
    assert_eq!(password.len(), 16);
}

#[test]
fn test_password_validation() {
    // Too short
    assert!(AuthService::validate_password_strength("Short1!").is_err());

    // Missing uppercase
    assert!(AuthService::validate_password_strength("lowercase123!").is_err());

    // Missing lowercase
    assert!(AuthService::validate_password_strength("UPPERCASE123!").is_err());

    // Missing digit
    assert!(AuthService::validate_password_strength("NoDigitsHere!").is_err());

    // Missing special
    assert!(AuthService::validate_password_strength("NoSpecial123").is_err());

    // Valid password
    assert!(AuthService::validate_password_strength("ValidPassword123!").is_ok());
}

#[test]
fn test_create_superadmin() {
    let (_dir, service) = create_test_service();

    let (user, password) = service
        .create_superadmin("default".to_string(), "admin".to_string())
        .unwrap();

    assert_eq!(user.username, "admin");
    assert!(user.must_change_password);
    assert!(user.access_flags.console_login);
    assert_eq!(password.len(), 16);

    // Verify password works
    assert!(AuthService::verify_password(&password, &user.password_hash).unwrap());
}

#[test]
fn test_authenticate() {
    let (_dir, service) = create_test_service();

    // Create a user
    let (user, password) = service
        .create_superadmin("default".to_string(), "testuser".to_string())
        .unwrap();

    // Authenticate successfully
    let (auth_user, token) = service
        .authenticate("default", "testuser", &password, AdminInterface::Console)
        .unwrap();

    assert_eq!(auth_user.username, "testuser");
    assert!(!token.is_empty());

    // Verify token
    let claims = service.validate_token(&token).unwrap();
    assert_eq!(claims.username, "testuser");
    assert_eq!(claims.tenant_id, "default");
}

#[test]
fn test_authenticate_wrong_password() {
    let (_dir, service) = create_test_service();

    service
        .create_superadmin("default".to_string(), "testuser".to_string())
        .unwrap();

    let result = service.authenticate(
        "default",
        "testuser",
        "wrong_password",
        AdminInterface::Console,
    );

    assert!(result.is_err());
}

#[test]
fn test_change_password() {
    let (_dir, service) = create_test_service();

    let (user, old_password) = service
        .create_superadmin("default".to_string(), "testuser".to_string())
        .unwrap();

    let new_password = "NewValidPassword123!";

    // Change password
    service
        .change_password("default", "testuser", &old_password, new_password)
        .unwrap();

    // Verify new password works
    let result = service.authenticate("default", "testuser", new_password, AdminInterface::Console);
    assert!(result.is_ok());

    // Verify old password doesn't work
    let result = service.authenticate(
        "default",
        "testuser",
        &old_password,
        AdminInterface::Console,
    );
    assert!(result.is_err());
}
