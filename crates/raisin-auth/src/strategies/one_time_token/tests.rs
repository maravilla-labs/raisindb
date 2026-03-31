use super::*;
use crate::strategy::{AuthCredentials, AuthStrategy};
use raisin_error::Error;
use raisin_models::auth::AuthProviderConfig;

#[test]
fn test_token_generation() {
    let (token, hash) =
        OneTimeTokenStrategy::generate_token("rdb_api").expect("Failed to generate token");

    assert!(token.starts_with("rdb_api_"));
    assert!(token.len() > 40);
    assert_eq!(hash.len(), 64);
    assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn test_token_generation_without_prefix() {
    let (token, hash) = OneTimeTokenStrategy::generate_token("").expect("Failed to generate token");

    assert!(!token.starts_with('_'));
    assert!(token.len() > 32);
    assert_eq!(hash.len(), 64);
}

#[test]
fn test_token_verification() {
    let (token, hash) =
        OneTimeTokenStrategy::generate_token("rdb_api").expect("Failed to generate token");

    assert!(OneTimeTokenStrategy::verify_token(&token, &hash));
    assert!(!OneTimeTokenStrategy::verify_token("wrong_token", &hash));
    assert!(!OneTimeTokenStrategy::verify_token("", &hash));
}

#[test]
fn test_hash_consistency() {
    let token = "rdb_api_test_token_12345";

    let hash1 = OneTimeTokenStrategy::hash_token(token);
    let hash2 = OneTimeTokenStrategy::hash_token(token);

    assert_eq!(hash1, hash2);
}

#[test]
fn test_different_tokens_produce_different_hashes() {
    let (token1, hash1) =
        OneTimeTokenStrategy::generate_token("rdb_api").expect("Failed to generate token");
    let (token2, hash2) =
        OneTimeTokenStrategy::generate_token("rdb_api").expect("Failed to generate token");

    assert_ne!(token1, token2);
    assert_ne!(hash1, hash2);
}

#[test]
fn test_extract_prefix() {
    assert_eq!(
        OneTimeTokenStrategy::extract_prefix("rdb_api_abc123"),
        "rdb"
    );
    assert_eq!(
        OneTimeTokenStrategy::extract_prefix("rdb_inv_xyz789"),
        "rdb"
    );
    assert_eq!(
        OneTimeTokenStrategy::extract_prefix("prefix_token"),
        "prefix"
    );
    assert_eq!(OneTimeTokenStrategy::extract_prefix("noprefix"), "");
}

#[test]
fn test_custom_token_length() {
    let (token, hash) = OneTimeTokenStrategy::generate_token_with_length("test", 64)
        .expect("Failed to generate token");

    assert!(token.len() > 80);
    assert_eq!(hash.len(), 64);
}

#[test]
fn test_minimum_token_length_validation() {
    let result = OneTimeTokenStrategy::generate_token_with_length("test", 8);

    assert!(result.is_err());
    match result {
        Err(Error::Validation(msg)) => {
            assert!(msg.contains("at least"));
            assert!(msg.contains("16 bytes"));
        }
        _ => panic!("Expected Validation error"),
    }
}

#[test]
fn test_constant_time_compare() {
    assert!(OneTimeTokenStrategy::constant_time_compare("abc", "abc"));
    assert!(!OneTimeTokenStrategy::constant_time_compare("abc", "def"));
    assert!(!OneTimeTokenStrategy::constant_time_compare("abc", "abcd"));
    assert!(OneTimeTokenStrategy::constant_time_compare("", ""));
}

#[test]
fn test_strategy_id() {
    let strategy = OneTimeTokenStrategy::new();
    assert_eq!(strategy.id().as_ref(), StrategyId::ONE_TIME_TOKEN);
    assert_eq!(strategy.name(), "One-Time Token Authentication");
}

#[test]
fn test_supports_credentials() {
    let strategy = OneTimeTokenStrategy::new();

    let ott_creds = AuthCredentials::OneTimeToken {
        token: "token123".to_string(),
    };
    assert!(strategy.supports(&ott_creds));

    let api_creds = AuthCredentials::ApiKey {
        key: "key123".to_string(),
    };
    assert!(strategy.supports(&api_creds));

    let magic_creds = AuthCredentials::MagicLinkToken {
        token: "magic123".to_string(),
    };
    assert!(strategy.supports(&magic_creds));

    let up_creds = AuthCredentials::UsernamePassword {
        username: "user@example.com".to_string(),
        password: "password".to_string(),
    };
    assert!(!strategy.supports(&up_creds));
}

#[tokio::test]
async fn test_init_no_op() {
    let mut strategy = OneTimeTokenStrategy::new();
    let config = AuthProviderConfig::local();

    let result = strategy.init(&config, None).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_authenticate_returns_skeleton_error() {
    let strategy = OneTimeTokenStrategy::new();
    let creds = AuthCredentials::OneTimeToken {
        token: "rdb_api_test123".to_string(),
    };

    let result = strategy.authenticate("tenant-1", creds).await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    match err {
        Error::Internal(msg) => {
            assert!(msg.contains("skeleton"));
            assert!(msg.contains("AuthService"));
        }
        _ => panic!("Expected Internal error"),
    }
}

#[tokio::test]
async fn test_authenticate_wrong_credentials_type() {
    let strategy = OneTimeTokenStrategy::new();
    let creds = AuthCredentials::UsernamePassword {
        username: "user@example.com".to_string(),
        password: "password".to_string(),
    };

    let result = strategy.authenticate("tenant-1", creds).await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    match err {
        Error::Validation(msg) => {
            assert!(msg.contains("token credentials"));
        }
        _ => panic!("Expected Validation error"),
    }
}

#[test]
fn test_token_randomness() {
    let mut tokens = std::collections::HashSet::new();

    for _ in 0..100 {
        let (token, _) =
            OneTimeTokenStrategy::generate_token("test").expect("Failed to generate token");
        tokens.insert(token);
    }

    assert_eq!(tokens.len(), 100);
}

#[test]
fn test_hash_token_edge_cases() {
    let hash1 = OneTimeTokenStrategy::hash_token("");
    let hash2 = OneTimeTokenStrategy::hash_token("");
    assert_eq!(hash1, hash2);
    assert_eq!(hash1.len(), 64);

    let long_token = "a".repeat(10000);
    let hash = OneTimeTokenStrategy::hash_token(&long_token);
    assert_eq!(hash.len(), 64);

    let unicode_token = "token";
    let hash = OneTimeTokenStrategy::hash_token(unicode_token);
    assert_eq!(hash.len(), 64);
}

#[test]
fn test_verify_token_timing_safety() {
    let (token, hash) =
        OneTimeTokenStrategy::generate_token("test").expect("Failed to generate token");

    assert!(OneTimeTokenStrategy::verify_token(&token, &hash));
    assert!(!OneTimeTokenStrategy::verify_token("wrong", &hash));
    assert!(!OneTimeTokenStrategy::verify_token(&token[..10], &hash));
}
