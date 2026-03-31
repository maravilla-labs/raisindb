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

//! User info claim mapping to authentication results.

use raisin_error::Result;
use std::collections::HashMap;

use crate::strategy::AuthenticationResult;

use super::OidcStrategy;

impl OidcStrategy {
    /// Map user info claims to an AuthenticationResult.
    ///
    /// Extracts user attributes from the provider's claims according to the
    /// configured attribute mapping.
    pub(super) fn map_user_info(
        &self,
        claims: HashMap<String, serde_json::Value>,
    ) -> Result<AuthenticationResult> {
        let config = self.get_config()?;
        let mapping = &config.attribute_mapping;

        let email = claims
            .get(&mapping.email_claim)
            .and_then(|v| v.as_str())
            .map(String::from);

        let display_name = claims
            .get(&mapping.name_claim)
            .and_then(|v| v.as_str())
            .map(String::from);

        let avatar_url = claims
            .get(&mapping.picture_claim)
            .and_then(|v| v.as_str())
            .map(String::from);

        let email_verified = claims
            .get(&mapping.email_verified_claim)
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Extract external ID (typically "sub" claim)
        let external_id = claims.get("sub").and_then(|v| v.as_str()).map(String::from);

        // Extract provider groups if configured
        let provider_groups = if let Some(groups_claim) = &config.groups_claim {
            claims
                .get(groups_claim)
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str())
                        .map(String::from)
                        .collect()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        let mut result =
            AuthenticationResult::new(self.strategy_id.clone()).with_email_verified(email_verified);

        if let Some(email) = email {
            result = result.with_email(email);
        }

        if let Some(name) = display_name {
            result = result.with_display_name(name);
        }

        if let Some(ext_id) = external_id {
            result = result.with_external_id(ext_id);
        }

        result.avatar_url = avatar_url;
        result.provider_claims = claims;
        result.provider_groups = provider_groups;

        Ok(result)
    }
}
