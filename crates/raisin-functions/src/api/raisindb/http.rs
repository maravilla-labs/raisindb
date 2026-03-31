// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! HTTP request operation implementation for RaisinFunctionApi

use raisin_error::Result;
use serde_json::Value;

use super::RaisinFunctionApi;

impl RaisinFunctionApi {
    pub(crate) async fn impl_http_request(
        &self,
        method: &str,
        url: &str,
        options: Value,
    ) -> Result<Value> {
        tracing::trace!(
            method = method,
            url = url,
            http_enabled = self.network_policy.http_enabled,
            allowed_urls = ?self.network_policy.allowed_urls,
            "http_request - checking policy"
        );

        // Check network policy
        if !self.is_url_allowed(url) {
            tracing::trace!(url = url, "http_request - URL BLOCKED by policy");
            return Err(raisin_error::Error::Validation(format!(
                "URL not allowed by network policy: {} (http_enabled: {}, allowed_urls: {:?})",
                url, self.network_policy.http_enabled, self.network_policy.allowed_urls
            )));
        }

        tracing::trace!("http_request - URL ALLOWED, proceeding with request");

        let callback = self.callbacks.http_request.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation("HTTP request callback not configured".to_string())
        })?;

        callback(method.to_string(), url.to_string(), options).await
    }
}
