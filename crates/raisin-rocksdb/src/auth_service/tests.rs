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

#[test]
fn user_token_lifetimes_follow_the_supplied_settings() {
    use raisin_models::auth::{Identity, Session, SessionSettings};
    use raisin_models::timestamp::StorageTimestamp;

    let (_dir, service) = create_test_service();
    let identity = Identity::new("id-1".into(), "t".into(), "a@example.com".into());
    let session = Session::new(
        "sid-1".into(),
        "t".into(),
        "id-1".into(),
        "local".into(),
        "fam".into(),
        StorageTimestamp::now(),
    );

    // Default path: unchanged one hour / thirty days.
    let tokens = service
        .generate_user_tokens(&identity, &session, None, None)
        .unwrap();
    assert_eq!(tokens.expires_in, 3600);
    assert_eq!(tokens.refresh_expires_in, Some(30 * 24 * 3600));
    let claims = service.validate_user_token(&tokens.access_token).unwrap();
    assert!((claims.exp - claims.iat - 3600).abs() <= 1);

    // Configured path: the stored session settings decide.
    let settings = SessionSettings {
        access_token_duration_seconds: 600,
        refresh_token_duration_seconds: 7200,
        ..SessionSettings::default()
    };
    let lifetimes = TokenLifetimes::from_session_settings(&settings);
    let tokens = service
        .generate_user_tokens_with_lifetimes(&identity, &session, None, None, lifetimes)
        .unwrap();
    assert_eq!(tokens.expires_in, 600);
    assert_eq!(tokens.refresh_expires_in, Some(7200));
    let claims = service.validate_user_token(&tokens.access_token).unwrap();
    assert!((claims.exp - claims.iat - 600).abs() <= 1);
    let refresh = service
        .validate_refresh_token(&tokens.refresh_token)
        .unwrap();
    assert!((refresh.exp - refresh.iat - 7200).abs() <= 1);

    // Zero is not a lifetime: it falls back to the default instead of minting
    // a token that is expired on arrival.
    let zero = TokenLifetimes {
        access_seconds: 0,
        refresh_seconds: 0,
    }
    .sanitized();
    assert_eq!(zero, TokenLifetimes::default());

    // Refresh honours the same lifetimes.
    let (refreshed, generation) = service
        .refresh_user_tokens_with_lifetimes(&identity, &session, &refresh, None, lifetimes)
        .unwrap();
    assert_eq!(generation, session.token_generation + 1);
    assert_eq!(refreshed.expires_in, 600);
}
