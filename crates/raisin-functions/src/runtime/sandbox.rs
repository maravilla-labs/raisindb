// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Sandbox utilities for safe function execution

use raisin_error::{Error, Result};
use std::time::Duration;
use tokio::time::timeout;

use crate::types::{NetworkPolicy, ResourceLimits};

/// Configuration for sandboxed execution
#[derive(Debug, Clone, Default)]
pub struct SandboxConfig {
    /// Resource limits
    pub limits: ResourceLimits,
    /// Network policy
    pub network: NetworkPolicy,
}

impl SandboxConfig {
    /// Create a new sandbox config
    pub fn new(limits: ResourceLimits, network: NetworkPolicy) -> Self {
        Self { limits, network }
    }
}

/// Sandbox for controlled function execution
pub struct Sandbox {
    config: SandboxConfig,
    http_client: Option<reqwest::Client>,
}

impl Sandbox {
    /// Create a new sandbox with the given configuration
    pub fn new(config: SandboxConfig) -> Self {
        let http_client = if config.network.http_enabled {
            Some(
                reqwest::Client::builder()
                    .timeout(Duration::from_millis(config.network.request_timeout_ms))
                    .build()
                    .expect("Failed to create HTTP client"),
            )
        } else {
            None
        };

        Self {
            config,
            http_client,
        }
    }

    /// Get timeout duration
    pub fn timeout(&self) -> Duration {
        Duration::from_millis(self.config.limits.timeout_ms)
    }

    /// Get memory limit
    pub fn memory_limit(&self) -> u64 {
        self.config.limits.max_memory_bytes
    }

    /// Get instruction limit
    pub fn instruction_limit(&self) -> Option<u64> {
        self.config.limits.max_instructions
    }

    /// Execute a future with timeout
    pub async fn execute_with_timeout<F, T>(&self, future: F) -> Result<T>
    where
        F: std::future::Future<Output = Result<T>>,
    {
        match timeout(self.timeout(), future).await {
            Ok(result) => result,
            Err(_) => Err(Error::InvalidState(format!(
                "Function execution timed out after {}ms",
                self.config.limits.timeout_ms
            ))),
        }
    }

    /// Check if a URL is allowed by the network policy
    pub fn is_url_allowed(&self, url: &str) -> bool {
        self.config.network.is_url_allowed(url)
    }

    /// Make an HTTP request (only to allowlisted URLs)
    pub async fn http_request(
        &self,
        method: &str,
        url: &str,
        body: Option<serde_json::Value>,
        headers: &std::collections::HashMap<String, String>,
    ) -> Result<HttpResponse> {
        // Check if HTTP is enabled
        if !self.config.network.http_enabled {
            return Err(Error::Validation(
                "HTTP requests are not enabled for this function".to_string(),
            ));
        }

        // Check URL against allowlist
        if !self.is_url_allowed(url) {
            return Err(Error::Validation(format!("URL not in allowlist: {}", url)));
        }

        let client = self
            .http_client
            .as_ref()
            .ok_or_else(|| Error::Validation("HTTP client not initialized".to_string()))?;

        let mut request = match method.to_uppercase().as_str() {
            "GET" => client.get(url),
            "POST" => client.post(url),
            "PUT" => client.put(url),
            "PATCH" => client.patch(url),
            "DELETE" => client.delete(url),
            _ => {
                return Err(Error::Validation(format!(
                    "Invalid HTTP method: {}",
                    method
                )))
            }
        };

        // Add headers
        for (key, value) in headers {
            request = request.header(key, value);
        }

        // Add body
        if let Some(body) = body {
            request = request.json(&body);
        }

        // Make request
        let response = request
            .send()
            .await
            .map_err(|e| Error::Backend(format!("HTTP request failed: {}", e)))?;

        let status = response.status().as_u16();
        let response_headers: std::collections::HashMap<String, String> = response
            .headers()
            .iter()
            .filter_map(|(k, v)| v.to_str().ok().map(|v| (k.to_string(), v.to_string())))
            .collect();

        // Check response size
        let content_length = response.content_length().unwrap_or(0);
        if content_length > self.config.network.max_response_size_bytes {
            return Err(Error::Validation(format!(
                "Response size {} exceeds limit {}",
                content_length, self.config.network.max_response_size_bytes
            )));
        }

        // Read body
        let body: serde_json::Value = response.json().await.unwrap_or(serde_json::Value::Null);

        Ok(HttpResponse {
            status,
            headers: response_headers,
            body,
        })
    }
}

/// HTTP response from sandboxed request
#[derive(Debug, Clone)]
pub struct HttpResponse {
    /// HTTP status code
    pub status: u16,
    /// Response headers
    pub headers: std::collections::HashMap<String, String>,
    /// Response body (as JSON)
    pub body: serde_json::Value,
}

impl HttpResponse {
    /// Convert to JSON value
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "status": self.status,
            "headers": self.headers,
            "body": self.body,
        })
    }

    /// Check if response was successful (2xx)
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sandbox_config_default() {
        let config = SandboxConfig::default();
        assert_eq!(config.limits.timeout_ms, 30_000);
        assert!(!config.network.http_enabled);
    }

    #[test]
    fn test_url_allowlist() {
        let config = SandboxConfig {
            limits: ResourceLimits::default(),
            network: NetworkPolicy::allow_urls(vec!["https://api.example.com/*".to_string()]),
        };
        let sandbox = Sandbox::new(config);

        assert!(sandbox.is_url_allowed("https://api.example.com/v1/test"));
        assert!(!sandbox.is_url_allowed("https://other.com/api"));
    }

    #[tokio::test]
    async fn test_timeout() {
        let config = SandboxConfig {
            limits: ResourceLimits::default().with_timeout_ms(100),
            network: NetworkPolicy::default(),
        };
        let sandbox = Sandbox::new(config);

        let result = sandbox
            .execute_with_timeout(async {
                tokio::time::sleep(Duration::from_millis(200)).await;
                Ok::<_, Error>(())
            })
            .await;

        assert!(result.is_err());
    }
}
