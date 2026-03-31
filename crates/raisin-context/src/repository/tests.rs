use super::*;
use std::sync::Arc;

#[test]
fn test_repository_context_creation() {
    let ctx = RepositoryContext::new("acme", "website");
    assert_eq!(ctx.tenant_id(), "acme");
    assert_eq!(ctx.repository_id(), "website");
    assert_eq!(ctx.storage_prefix(), "/acme/repo/website");
}

#[test]
fn test_repository_context_default_tenant() {
    let ctx = RepositoryContext::new("default", "my-app");
    assert_eq!(ctx.tenant_id(), "default");
    assert_eq!(ctx.repository_id(), "my-app");
    assert_eq!(ctx.storage_prefix(), "/default/repo/my-app");
}

#[test]
fn test_workspace_prefix() {
    let ctx = RepositoryContext::new("acme", "website");
    assert_eq!(
        ctx.workspace_prefix("main"),
        "/acme/repo/website/workspace/main"
    );
}

#[test]
fn test_branch_prefix() {
    let ctx = RepositoryContext::new("acme", "website");
    assert_eq!(
        ctx.branch_prefix("production"),
        "/acme/repo/website/branch/production"
    );
}

#[test]
fn test_node_key() {
    let ctx = RepositoryContext::new("acme", "website");
    assert_eq!(
        ctx.node_key("develop", "main", "article-123"),
        "/acme/repo/website/branch/develop/workspace/main/nodes/article-123"
    );
}

#[test]
fn test_workspace_scope() {
    let ctx = Arc::new(RepositoryContext::new("acme", "website"));
    let scope = WorkspaceScope::new(ctx, "main")
        .with_branch("develop")
        .with_revision(42);

    assert_eq!(scope.workspace_id, "main");
    assert_eq!(scope.branch, Some("develop".to_string()));
    assert_eq!(scope.as_of_revision, Some(42));
    assert_eq!(scope.effective_branch("production"), "develop");
}

#[test]
fn test_workspace_scope_defaults() {
    let ctx = Arc::new(RepositoryContext::new("acme", "website"));
    let scope = WorkspaceScope::new(ctx, "main");

    assert_eq!(scope.branch, None);
    assert_eq!(scope.as_of_revision, None);
    assert_eq!(scope.effective_branch("production"), "production");
}

#[test]
fn test_repository_config_validate_locale_fallback_chains() {
    let mut config = RepositoryConfig::default();
    config.supported_languages = vec!["en".to_string(), "fr".to_string(), "fr-CA".to_string()];

    // Valid configuration
    let mut chains = std::collections::HashMap::new();
    chains.insert(
        "fr-CA".to_string(),
        vec!["fr".to_string(), "en".to_string()],
    );
    config.locale_fallback_chains = chains;

    assert!(config.validate_locale_fallback_chains().is_ok());
}

#[test]
fn test_repository_config_validate_locale_fallback_chains_invalid_locale() {
    let mut config = RepositoryConfig::default();
    config.supported_languages = vec!["en".to_string(), "fr".to_string()];

    // Invalid: de-DE not in supported_languages
    let mut chains = std::collections::HashMap::new();
    chains.insert(
        "de-DE".to_string(),
        vec!["de".to_string(), "en".to_string()],
    );
    config.locale_fallback_chains = chains;

    assert!(config.validate_locale_fallback_chains().is_err());
}

#[test]
fn test_repository_config_validate_locale_fallback_chains_circular() {
    let mut config = RepositoryConfig::default();
    config.supported_languages = vec!["en".to_string(), "fr".to_string()];

    // Invalid: circular reference
    let mut chains = std::collections::HashMap::new();
    chains.insert("fr".to_string(), vec!["fr".to_string(), "en".to_string()]);
    config.locale_fallback_chains = chains;

    assert!(config.validate_locale_fallback_chains().is_err());
}

#[test]
fn test_repository_config_get_fallback_chain_explicit() {
    let mut config = RepositoryConfig::default();
    config.supported_languages = vec!["en".to_string(), "fr".to_string(), "fr-CA".to_string()];

    // Explicit configuration
    let mut chains = std::collections::HashMap::new();
    chains.insert(
        "fr-CA".to_string(),
        vec!["fr".to_string(), "en".to_string()],
    );
    config.locale_fallback_chains = chains;

    let chain = config.get_fallback_chain("fr-CA");
    assert_eq!(
        chain,
        vec!["fr-CA".to_string(), "fr".to_string(), "en".to_string()]
    );
}

#[test]
fn test_repository_config_get_fallback_chain_automatic() {
    let config = RepositoryConfig::default();

    // Automatic fallback for locale with region
    let chain = config.get_fallback_chain("fr-CA");
    assert_eq!(
        chain,
        vec!["fr-CA".to_string(), "fr".to_string(), "en".to_string()]
    );

    // Automatic fallback for language only
    let chain = config.get_fallback_chain("fr");
    assert_eq!(chain, vec!["fr".to_string(), "en".to_string()]);

    // No fallback for default language
    let chain = config.get_fallback_chain("en");
    assert_eq!(chain, vec!["en".to_string()]);
}

#[test]
fn test_repository_config_default_language_in_supported() {
    let config = RepositoryConfig::default();
    assert!(config
        .supported_languages
        .contains(&config.default_language));
    assert!(config.validate_locale_fallback_chains().is_ok());
}
