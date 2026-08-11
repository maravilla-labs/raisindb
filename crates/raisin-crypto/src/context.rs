//! Binding context (AES-GCM additional authenticated data) for sealed secrets.
//!
//! # Why the fields are private
//!
//! A [`SecretContext`] is only constructible through the family constructors
//! below. That is deliberate and is the single most important design decision
//! in this module.
//!
//! Across the server there are read sites that swallow a decrypt failure with
//! `.ok()` and carry on — "no credential, proceed unauthenticated". If callers
//! could hand-write a context, a reader whose context drifted by one character
//! from the writer's would not raise anything: the open would fail, the `.ok()`
//! would turn it into `None`, and the system would silently behave as if the
//! secret were absent. Forcing both sides through the same named constructor
//! makes that drift a compile-time impossibility rather than a silent runtime
//! downgrade.
//!
//! # Why the AAD is length-prefixed
//!
//! Components are encoded as `[u32 BE len][bytes]` each, concatenated — not
//! joined with a separator. A `\0`-join is ambiguous: `tenant="a\0b", repo=""`
//! and `tenant="a", repo="b"` would produce identical AAD, so a blob from one
//! would open under the other. AAD is never stored, so the 16 extra bytes cost
//! nothing at rest. NUL bytes are additionally rejected at construction.

use crate::error::{CryptoError, Result};

/// What to do when [`crate::SecretBox::open`] is handed a v1 (context-free)
/// blob.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum V1Policy {
    /// Open it, ignoring the context (v1 has no AAD). Required for every
    /// family that has data at rest from before envelope v2.
    Accept,
    /// Refuse with [`CryptoError::V1Rejected`]. Only safe for families whose
    /// data is all v2 — i.e. new families, or ones with a completed migration.
    Reject,
}

/// The authenticated binding for a sealed secret: which tenant, which repo,
/// which subsystem, which field.
///
/// Construct one only via the family constructors ([`SecretContext::ai_provider`]
/// and friends). Both the writer and the reader of a given secret must call the
/// *same* constructor with the *same* arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretContext {
    tenant: String,
    repo: String,
    scope: String,
    field: String,
    v1_policy: V1Policy,
}

fn check(component: &'static str, value: &str) -> Result<()> {
    if value.contains('\0') {
        return Err(CryptoError::InvalidContext { component });
    }
    Ok(())
}

impl SecretContext {
    fn build(
        tenant: impl Into<String>,
        repo: impl Into<String>,
        scope: impl Into<String>,
        field: impl Into<String>,
        v1_policy: V1Policy,
    ) -> Result<Self> {
        let ctx = Self {
            tenant: tenant.into(),
            repo: repo.into(),
            scope: scope.into(),
            field: field.into(),
            v1_policy,
        };
        check("tenant", &ctx.tenant)?;
        check("repo", &ctx.repo)?;
        check("scope", &ctx.scope)?;
        check("field", &ctx.field)?;
        Ok(ctx)
    }

    // ---- families -------------------------------------------------------

    /// An AI provider's API key. `provider` is the provider slug (`openai`, …).
    pub fn ai_provider(tenant: impl Into<String>, provider: impl Into<String>) -> Result<Self> {
        Self::build(tenant, "", "ai_provider", provider, V1Policy::Accept)
    }

    /// The tenant-wide embedding configuration's API key.
    pub fn embedding_config(tenant: impl Into<String>) -> Result<Self> {
        Self::build(tenant, "", "embedding_config", "api_key", V1Policy::Accept)
    }

    /// An integration connection's OAuth token bundle.
    pub fn integration_tokens(tenant: impl Into<String>, repo: impl Into<String>) -> Result<Self> {
        Self::build(tenant, repo, "integration", "tokens", V1Policy::Accept)
    }

    /// An integration's OAuth client secret.
    pub fn integration_client_secret(
        tenant: impl Into<String>,
        repo: impl Into<String>,
    ) -> Result<Self> {
        Self::build(
            tenant,
            repo,
            "integration",
            "client_secret",
            V1Policy::Accept,
        )
    }

    /// Secret-marked fields of an integration's config node.
    pub fn integration_config_secrets(
        tenant: impl Into<String>,
        repo: impl Into<String>,
    ) -> Result<Self> {
        Self::build(
            tenant,
            repo,
            "integration",
            "config_secrets",
            V1Policy::Accept,
        )
    }

    /// Secret-marked fields of a connector connection node.
    pub fn connection_secrets(tenant: impl Into<String>, repo: impl Into<String>) -> Result<Self> {
        Self::build(tenant, repo, "connection", "secrets", V1Policy::Accept)
    }

    /// An outbound MCP connection's OAuth token bundle.
    pub fn mcp_tokens(tenant: impl Into<String>, repo: impl Into<String>) -> Result<Self> {
        Self::build(tenant, repo, "mcp", "tokens", V1Policy::Accept)
    }

    /// A static credential (bearer/header) for an outbound MCP connection.
    pub fn mcp_credential(tenant: impl Into<String>, repo: impl Into<String>) -> Result<Self> {
        Self::build(tenant, repo, "mcp", "credential", V1Policy::Accept)
    }

    /// An authserver authorization code. Not tenant- or repo-scoped: the code
    /// is opaque and is resolved before a tenant is known.
    ///
    /// Defaults to [`V1Policy::Reject`] — codes are short-lived, so there is no
    /// meaningful body of v1 data at rest to preserve.
    pub fn authorization_code() -> Result<Self> {
        Self::build("", "", "authserver", "authorization_code", V1Policy::Reject)
    }

    /// A value in the node secret store (`secret://` references).
    ///
    /// Defaults to [`V1Policy::Reject`]: this family is new, so every blob it
    /// will ever see is v2 and accepting v1 would only widen the attack
    /// surface.
    pub fn node_secret(tenant: impl Into<String>, repo: impl Into<String>) -> Result<Self> {
        Self::build(tenant, repo, "node_secret", "value", V1Policy::Reject)
    }

    // ---- accessors ------------------------------------------------------

    /// Override the v1 policy (e.g. to tighten a family after a migration).
    pub fn with_v1_policy(mut self, policy: V1Policy) -> Self {
        self.v1_policy = policy;
        self
    }

    pub fn v1_policy(&self) -> V1Policy {
        self.v1_policy
    }

    pub fn scope(&self) -> &str {
        &self.scope
    }

    /// The additional authenticated data for this context: each component
    /// encoded as `[u32 BE length][bytes]`, in the fixed order
    /// tenant, repo, scope, field.
    pub fn aad(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(
            16 + self.tenant.len() + self.repo.len() + self.scope.len() + self.field.len(),
        );
        for part in [&self.tenant, &self.repo, &self.scope, &self.field] {
            out.extend_from_slice(&(part.len() as u32).to_be_bytes());
            out.extend_from_slice(part.as_bytes());
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rejects_nul_in_any_component() {
        assert!(matches!(
            SecretContext::integration_tokens("a\0b", "repo"),
            Err(CryptoError::InvalidContext {
                component: "tenant"
            })
        ));
        assert!(matches!(
            SecretContext::integration_tokens("a", "re\0po"),
            Err(CryptoError::InvalidContext { component: "repo" })
        ));
        assert!(matches!(
            SecretContext::ai_provider("a", "open\0ai"),
            Err(CryptoError::InvalidContext { component: "field" })
        ));
    }

    #[test]
    fn test_aad_is_unambiguous_across_component_boundaries() {
        // With a `\0`-join these two would be byte-identical.
        let a = SecretContext::integration_tokens("a", "b").unwrap();
        let b = SecretContext::integration_tokens("ab", "").unwrap();
        assert_ne!(a.aad(), b.aad());
    }

    #[test]
    fn test_aad_layout_is_length_prefixed() {
        let ctx = SecretContext::integration_tokens("t", "r").unwrap();
        let aad = ctx.aad();
        assert_eq!(&aad[0..4], &[0, 0, 0, 1]);
        assert_eq!(aad[4], b't');
        assert_eq!(&aad[5..9], &[0, 0, 0, 1]);
        assert_eq!(aad[9], b'r');
    }

    #[test]
    fn test_family_defaults() {
        assert_eq!(
            SecretContext::integration_tokens("t", "r")
                .unwrap()
                .v1_policy(),
            V1Policy::Accept
        );
        assert_eq!(
            SecretContext::node_secret("t", "r").unwrap().v1_policy(),
            V1Policy::Reject
        );
        assert_eq!(
            SecretContext::authorization_code().unwrap().v1_policy(),
            V1Policy::Reject
        );
    }
}
