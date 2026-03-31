// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Configuration types for function execution

use serde::{Deserialize, Serialize};

/// Resource limits for function execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceLimits {
    /// Maximum execution time in milliseconds (default: 30,000 = 30 seconds)
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,

    /// Maximum memory in bytes (default: 128MB)
    #[serde(default = "default_memory")]
    pub max_memory_bytes: u64,

    /// Maximum CPU instructions for QuickJS (None = unlimited)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_instructions: Option<u64>,

    /// Maximum stack size in bytes (default: 1MB)
    #[serde(default = "default_stack")]
    pub max_stack_bytes: u64,
}

fn default_timeout() -> u64 {
    30_000 // 30 seconds
}

fn default_memory() -> u64 {
    128 * 1024 * 1024 // 128MB
}

fn default_stack() -> u64 {
    1024 * 1024 // 1MB
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            timeout_ms: default_timeout(),
            max_memory_bytes: default_memory(),
            max_instructions: Some(100_000_000), // 100M instructions
            max_stack_bytes: default_stack(),
        }
    }
}

impl ResourceLimits {
    /// Create minimal resource limits for quick functions
    pub fn minimal() -> Self {
        Self {
            timeout_ms: 5_000,                  // 5 seconds
            max_memory_bytes: 32 * 1024 * 1024, // 32MB
            max_instructions: Some(10_000_000), // 10M instructions
            max_stack_bytes: 512 * 1024,        // 512KB
        }
    }

    /// Create generous resource limits for complex functions
    pub fn generous() -> Self {
        Self {
            timeout_ms: 300_000,                   // 5 minutes
            max_memory_bytes: 512 * 1024 * 1024,   // 512MB
            max_instructions: Some(1_000_000_000), // 1B instructions
            max_stack_bytes: 4 * 1024 * 1024,      // 4MB
        }
    }

    /// Set timeout
    pub fn with_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = timeout_ms;
        self
    }

    /// Set memory limit
    pub fn with_memory_bytes(mut self, bytes: u64) -> Self {
        self.max_memory_bytes = bytes;
        self
    }

    /// Set instruction limit
    pub fn with_instructions(mut self, instructions: u64) -> Self {
        self.max_instructions = Some(instructions);
        self
    }

    /// Remove instruction limit
    pub fn unlimited_instructions(mut self) -> Self {
        self.max_instructions = None;
        self
    }
}

/// Network access policy for functions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkPolicy {
    /// Whether HTTP requests are allowed
    #[serde(default)]
    pub http_enabled: bool,

    /// Allowlisted URL patterns (glob-style)
    /// Examples: "https://api.example.com/*", "https://*.myservice.com/api/*"
    #[serde(default)]
    pub allowed_urls: Vec<String>,

    /// Maximum concurrent HTTP requests
    #[serde(default = "default_concurrent_requests")]
    pub max_concurrent_requests: u32,

    /// Request timeout in milliseconds
    #[serde(default = "default_request_timeout")]
    pub request_timeout_ms: u64,

    /// Maximum response body size in bytes
    #[serde(default = "default_max_response_size")]
    pub max_response_size_bytes: u64,
}

fn default_concurrent_requests() -> u32 {
    5
}

fn default_request_timeout() -> u64 {
    10_000 // 10 seconds
}

fn default_max_response_size() -> u64 {
    10 * 1024 * 1024 // 10MB
}

impl Default for NetworkPolicy {
    fn default() -> Self {
        Self {
            http_enabled: false,
            allowed_urls: Vec::new(),
            max_concurrent_requests: default_concurrent_requests(),
            request_timeout_ms: default_request_timeout(),
            max_response_size_bytes: default_max_response_size(),
        }
    }
}

impl NetworkPolicy {
    /// Create a policy that allows no network access
    pub fn no_network() -> Self {
        Self::default()
    }

    /// Create a policy that allows specific URLs
    pub fn allow_urls(urls: Vec<String>) -> Self {
        Self {
            http_enabled: true,
            allowed_urls: urls,
            ..Default::default()
        }
    }

    /// Enable HTTP access
    pub fn with_http_enabled(mut self, enabled: bool) -> Self {
        self.http_enabled = enabled;
        self
    }

    /// Add allowed URL pattern
    pub fn with_allowed_url(mut self, pattern: impl Into<String>) -> Self {
        self.http_enabled = true;
        self.allowed_urls.push(pattern.into());
        self
    }

    /// Set max concurrent requests
    pub fn with_max_concurrent(mut self, max: u32) -> Self {
        self.max_concurrent_requests = max;
        self
    }

    /// Set request timeout
    pub fn with_request_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.request_timeout_ms = timeout_ms;
        self
    }

    /// Check if a URL is allowed by this policy
    pub fn is_url_allowed(&self, url: &str) -> bool {
        if !self.http_enabled {
            return false;
        }

        if self.allowed_urls.is_empty() {
            return false;
        }

        self.allowed_urls.iter().any(|pattern| {
            glob::Pattern::new(pattern)
                .map(|p| p.matches(url))
                .unwrap_or(false)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_allowlist() {
        let policy = NetworkPolicy::allow_urls(vec![
            "https://api.example.com/*".to_string(),
            "https://*.myservice.com/api/*".to_string(),
        ]);

        assert!(policy.is_url_allowed("https://api.example.com/v1/users"));
        assert!(policy.is_url_allowed("https://api.example.com/anything"));
        assert!(!policy.is_url_allowed("https://other.com/api"));

        // Glob patterns with wildcards
        assert!(policy.is_url_allowed("https://foo.myservice.com/api/test"));
        assert!(!policy.is_url_allowed("https://foo.myservice.com/other"));
    }

    #[test]
    fn test_no_network_policy() {
        let policy = NetworkPolicy::no_network();
        assert!(!policy.is_url_allowed("https://api.example.com/anything"));
    }
}
