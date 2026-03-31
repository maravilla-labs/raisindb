// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Tests for authentication configuration.

#[cfg(test)]
mod tests {
    use crate::auth::config::{AuthProviderConfig, PasswordPolicy, TenantAuthConfig};

    #[test]
    fn test_password_policy_validation() {
        let policy = PasswordPolicy::default();

        // Valid password
        assert!(policy.validate("SecurePass123").is_ok());

        // Too short
        let result = policy.validate("Short1");
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| e.contains("at least")));

        // Missing uppercase
        let result = policy.validate("lowercase123");
        assert!(result.is_err());

        // Missing lowercase
        let result = policy.validate("UPPERCASE123");
        assert!(result.is_err());

        // Missing digit
        let result = policy.validate("NoDigitsHere");
        assert!(result.is_err());
    }

    #[test]
    fn test_auth_provider_config() {
        let local = AuthProviderConfig::local();
        assert_eq!(local.strategy_id, "local");
        assert!(local.enabled);
        assert!(!local.is_oidc());

        let google = AuthProviderConfig::google("client-id".to_string());
        assert!(google.is_oidc());
        assert_eq!(
            google.issuer_url,
            Some("https://accounts.google.com".to_string())
        );
    }

    #[test]
    fn test_tenant_auth_config() {
        let mut config = TenantAuthConfig::new("tenant-1".to_string());
        config.providers.push(AuthProviderConfig::local());
        config.providers.push(AuthProviderConfig::magic_link());

        assert!(config.local_auth_enabled());
        assert!(config.magic_link_enabled());

        let providers: Vec<_> = config.enabled_providers().collect();
        assert_eq!(providers.len(), 2);
    }
}
