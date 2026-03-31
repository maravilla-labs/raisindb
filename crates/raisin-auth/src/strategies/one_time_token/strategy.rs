// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! `AuthStrategy` trait implementation for `OneTimeTokenStrategy`.

use async_trait::async_trait;
use raisin_error::{Error, Result};
use raisin_models::auth::AuthProviderConfig;

use crate::strategy::{AuthCredentials, AuthStrategy, AuthenticationResult, StrategyId};

use super::OneTimeTokenStrategy;

#[async_trait]
impl AuthStrategy for OneTimeTokenStrategy {
    fn id(&self) -> &StrategyId {
        &self.strategy_id
    }

    fn name(&self) -> &str {
        "One-Time Token Authentication"
    }

    async fn init(
        &mut self,
        _config: &AuthProviderConfig,
        _decrypted_secret: Option<&str>,
    ) -> Result<()> {
        Ok(())
    }

    async fn authenticate(
        &self,
        tenant_id: &str,
        credentials: AuthCredentials,
    ) -> Result<AuthenticationResult> {
        let token = match credentials {
            AuthCredentials::OneTimeToken { token } => token,
            AuthCredentials::ApiKey { key } => key,
            AuthCredentials::MagicLinkToken { token } => token,
            _ => {
                return Err(Error::Validation(
                    "One-time token strategy requires token credentials".to_string(),
                ))
            }
        };

        let _ = (tenant_id, token);

        Err(Error::internal(
            "OneTimeTokenStrategy::authenticate is a skeleton - actual authentication \
             is handled by AuthService which performs token lookup, validation, \
             and identity resolution using this strategy's helper methods"
                .to_string(),
        ))
    }

    fn supports(&self, credentials: &AuthCredentials) -> bool {
        matches!(
            credentials,
            AuthCredentials::OneTimeToken { .. }
                | AuthCredentials::ApiKey { .. }
                | AuthCredentials::MagicLinkToken { .. }
        )
    }
}
