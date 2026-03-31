use super::*;
use crate::strategy::{AuthCredentials, AuthStrategy};
use raisin_error::Error;
use raisin_models::auth::AuthProviderConfig;
use std::collections::HashMap;

#[test]
fn test_strategy_id() {
    let strategy = OidcStrategy::new("google", "Sign in with Google");
    assert!(strategy.id().is_oidc());
    assert_eq!(strategy.id().provider_name(), Some("google"));
    assert_eq!(strategy.name(), "Sign in with Google");
}

#[test]
fn test_pkce_code_verifier_generation() {
    let verifier1 = OidcStrategy::generate_code_verifier();
    let verifier2 = OidcStrategy::generate_code_verifier();

    assert_ne!(verifier1, verifier2);
    assert!(!verifier1.contains('+'));
    assert!(!verifier1.contains('/'));
    assert!(!verifier1.contains('='));
}

#[test]
fn test_pkce_code_challenge_generation() {
    let verifier = "test_verifier_1234567890";
    let challenge = OidcStrategy::generate_code_challenge(verifier);

    assert!(!challenge.contains('+'));
    assert!(!challenge.contains('/'));
    assert!(!challenge.contains('='));

    let challenge2 = OidcStrategy::generate_code_challenge(verifier);
    assert_eq!(challenge, challenge2);

    let challenge3 = OidcStrategy::generate_code_challenge("different_verifier");
    assert_ne!(challenge, challenge3);
}

#[test]
fn test_supports_credentials() {
    let strategy = OidcStrategy::new("google", "Sign in with Google");

    let code_creds = AuthCredentials::OAuth2Code {
        code: "auth_code".to_string(),
        state: "state123".to_string(),
        redirect_uri: "https://example.com/callback".to_string(),
    };
    assert!(strategy.supports(&code_creds));

    let refresh_creds = AuthCredentials::OAuth2RefreshToken {
        refresh_token: "refresh_token".to_string(),
    };
    assert!(strategy.supports(&refresh_creds));

    let local_creds = AuthCredentials::UsernamePassword {
        username: "user".to_string(),
        password: "pass".to_string(),
    };
    assert!(!strategy.supports(&local_creds));

    let magic_creds = AuthCredentials::MagicLinkToken {
        token: "token123".to_string(),
    };
    assert!(!strategy.supports(&magic_creds));
}

#[tokio::test]
async fn test_init_requires_client_id() {
    let mut strategy = OidcStrategy::new("google", "Sign in with Google");
    let mut config = AuthProviderConfig::google("".to_string());
    config.client_id = None;

    let result = strategy.init(&config, Some("secret")).await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    match err {
        Error::Validation(msg) => {
            assert!(msg.contains("client_id"));
        }
        _ => panic!("Expected Validation error"),
    }
}

#[tokio::test]
async fn test_init_requires_client_secret() {
    let mut strategy = OidcStrategy::new("google", "Sign in with Google");
    let config = AuthProviderConfig::google("client-id".to_string());

    let result = strategy.init(&config, None).await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    match err {
        Error::Validation(msg) => {
            assert!(msg.contains("client_secret"));
        }
        _ => panic!("Expected Validation error"),
    }
}

#[tokio::test]
async fn test_init_requires_endpoints_or_issuer() {
    let mut strategy = OidcStrategy::new("custom", "Custom Provider");
    let mut config = AuthProviderConfig::google("client-id".to_string());
    config.issuer_url = None;
    config.authorization_url = None;

    let result = strategy.init(&config, Some("secret")).await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    match err {
        Error::Validation(msg) => {
            assert!(msg.contains("issuer_url") || msg.contains("authorization_url"));
        }
        _ => panic!("Expected Validation error"),
    }
}

#[tokio::test]
async fn test_init_with_manual_endpoints() {
    let mut strategy = OidcStrategy::new("custom", "Custom Provider");
    let mut config = AuthProviderConfig::google("client-id".to_string());
    config.issuer_url = None;
    config.authorization_url = Some("https://auth.example.com/authorize".to_string());
    config.token_url = Some("https://auth.example.com/token".to_string());
    config.userinfo_url = Some("https://auth.example.com/userinfo".to_string());

    let result = strategy.init(&config, Some("secret")).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_get_config_before_init_fails() {
    let strategy = OidcStrategy::new("google", "Sign in with Google");

    let result = strategy.get_config();
    assert!(result.is_err());
    let err = result.unwrap_err();
    match err {
        Error::InvalidState(msg) => {
            assert!(msg.contains("not initialized"));
        }
        _ => panic!("Expected InvalidState error"),
    }
}

#[tokio::test]
async fn test_build_authorization_url() {
    let mut strategy = OidcStrategy::new("custom", "Custom Provider");
    let mut config = AuthProviderConfig::google("test-client-id".to_string());
    config.issuer_url = None;
    config.authorization_url = Some("https://auth.example.com/authorize".to_string());
    config.token_url = Some("https://auth.example.com/token".to_string());
    config.userinfo_url = Some("https://auth.example.com/userinfo".to_string());
    config.scopes = vec!["openid".to_string(), "email".to_string()];

    strategy.init(&config, Some("secret")).await.unwrap();

    let url = strategy
        .build_authorization_url(
            "https://app.example.com/callback",
            "state123",
            "challenge456",
            Some("nonce789"),
        )
        .unwrap();

    assert!(url.contains("client_id=test-client-id"));
    assert!(url.contains("response_type=code"));
    assert!(url.contains("redirect_uri=https%3A%2F%2Fapp.example.com%2Fcallback"));
    assert!(url.contains("scope=openid+email"));
    assert!(url.contains("state=state123"));
    assert!(url.contains("code_challenge=challenge456"));
    assert!(url.contains("code_challenge_method=S256"));
    assert!(url.contains("nonce=nonce789"));
}

#[tokio::test]
async fn test_authenticate_wrong_credentials_type() {
    let mut strategy = OidcStrategy::new("custom", "Custom Provider");
    let mut config = AuthProviderConfig::google("client-id".to_string());
    config.issuer_url = None;
    config.authorization_url = Some("https://auth.example.com/authorize".to_string());
    config.token_url = Some("https://auth.example.com/token".to_string());
    config.userinfo_url = Some("https://auth.example.com/userinfo".to_string());

    strategy.init(&config, Some("secret")).await.unwrap();

    let creds = AuthCredentials::UsernamePassword {
        username: "user".to_string(),
        password: "pass".to_string(),
    };

    let result = strategy.authenticate("tenant-1", creds).await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    match err {
        Error::Validation(msg) => {
            assert!(msg.contains("OAuth2"));
        }
        _ => panic!("Expected Validation error"),
    }
}

#[tokio::test]
async fn test_map_user_info() {
    let mut strategy = OidcStrategy::new("custom", "Custom Provider");
    let mut config = AuthProviderConfig::google("client-id".to_string());
    config.issuer_url = None;
    config.authorization_url = Some("https://auth.example.com/authorize".to_string());
    config.token_url = Some("https://auth.example.com/token".to_string());
    config.userinfo_url = Some("https://auth.example.com/userinfo".to_string());
    config.groups_claim = Some("groups".to_string());

    strategy.init(&config, Some("secret")).await.unwrap();

    let mut claims = HashMap::new();
    claims.insert("sub".to_string(), serde_json::json!("user-123"));
    claims.insert("email".to_string(), serde_json::json!("user@example.com"));
    claims.insert("name".to_string(), serde_json::json!("Test User"));
    claims.insert(
        "picture".to_string(),
        serde_json::json!("https://example.com/avatar.jpg"),
    );
    claims.insert("email_verified".to_string(), serde_json::json!(true));
    claims.insert("groups".to_string(), serde_json::json!(["admin", "users"]));

    let result = strategy.map_user_info(claims).unwrap();

    assert_eq!(result.email, Some("user@example.com".to_string()));
    assert_eq!(result.display_name, Some("Test User".to_string()));
    assert_eq!(
        result.avatar_url,
        Some("https://example.com/avatar.jpg".to_string())
    );
    assert_eq!(result.external_id, Some("user-123".to_string()));
    assert!(result.email_verified);
    assert_eq!(result.provider_groups, vec!["admin", "users"]);
}

#[tokio::test]
async fn test_handle_callback_missing_parameters() {
    let mut strategy = OidcStrategy::new("custom", "Custom Provider");
    let mut config = AuthProviderConfig::google("client-id".to_string());
    config.issuer_url = None;
    config.authorization_url = Some("https://auth.example.com/authorize".to_string());
    config.token_url = Some("https://auth.example.com/token".to_string());
    config.userinfo_url = Some("https://auth.example.com/userinfo".to_string());

    strategy.init(&config, Some("secret")).await.unwrap();

    let params = HashMap::new();

    let result = strategy.handle_callback("tenant-1", params).await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    match err {
        Error::Validation(msg) => {
            assert!(msg.contains("code"));
        }
        _ => panic!("Expected Validation error"),
    }
}
