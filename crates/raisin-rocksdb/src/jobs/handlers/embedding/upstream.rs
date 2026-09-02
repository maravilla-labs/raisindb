//! Which upstream an embedding job is about to call — the circuit breaker's key.
//!
//! # Why not `embedder_hash`
//!
//! The obvious candidate is [`EmbedderId::to_key_hash`], which the handler
//! already derives and logs, and which is stable and collision-free. But it
//! answers a different question. `embedder_hash` is a STORAGE partition
//! identity: it is `{provider, model, dimensions}`, because two models' vectors
//! are incomparable and must never share an index.
//!
//! A breaker key is a FAILURE DOMAIN identity, and the thing that fails is the
//! endpoint. Keying on the embedder hash would give one host serving three
//! models three separate breakers, each having to independently rediscover the
//! same outage with its own full run of failures — a smaller copy of exactly
//! the "every job rediscovers the outage alone" problem the breaker exists to
//! remove. The model is not part of what went down.
//!
//! So the key is `{wire protocol}:{endpoint}`:
//!
//! * the PROVIDER VARIANT, not a tenant's free-form provider slug. Two tenants
//!   naming the same gateway `marvel` and `infomaniak` must land on one
//!   breaker, and the variant is the same value `EmbedderId` uses for the same
//!   reason (see `ResolvedEmbeddingProvider::embedder_id`).
//! * the ENDPOINT, normalised to scheme + host + port. Two tenants pointed at
//!   `https://host/v1` and `https://host/v1/` share a failure domain and must
//!   share a breaker; a vendor default (`base_url: None`) is itself a stable
//!   endpoint identity and is spelled as such.
//!
//! # What is deliberately NOT in the key
//!
//! The API KEY, and anything else tenant-scoped. That is the whole design: an
//! upstream outage is not a tenant's fault and every tenant must learn it at
//! once. The converse risk — one tenant's expired credential parking everyone —
//! is handled by never opening the breaker on a 4xx
//! (`Error::is_upstream_fault`), not by narrowing the key.

use raisin_embeddings::resolve::ResolvedEmbeddingProvider;

/// The breaker key for a resolved embedding provider.
///
/// Prefixed `embedding:` because the registry is process-wide and will hold
/// other kinds of upstream (an image tower, a chat provider) beside these.
pub(crate) fn upstream_key(resolved: &ResolvedEmbeddingProvider) -> String {
    format!(
        "embedding:{}:{}",
        format!("{:?}", resolved.provider).to_lowercase(),
        endpoint_identity(resolved.base_url.as_deref())
    )
}

/// Scheme + host + port of a base URL, or a stable marker for "the vendor's own
/// host".
///
/// Parsed rather than string-matched so a trailing slash, a path, or a query
/// cannot split one endpoint into two breakers. A URL that will not parse falls
/// back to the raw string: two tenants with the same unparseable value still
/// share a breaker, which is the property that matters, and the value is a
/// breaker key rather than something dialled.
fn endpoint_identity(base_url: Option<&str>) -> String {
    let Some(raw) = base_url else {
        return "vendor-default".to_string();
    };
    match url::Url::parse(raw) {
        Ok(url) => match (url.host_str(), url.port_or_known_default()) {
            (Some(host), Some(port)) => format!("{}://{}:{}", url.scheme(), host, port),
            (Some(host), None) => format!("{}://{}", url.scheme(), host),
            _ => raw.to_string(),
        },
        Err(_) => raw.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_embeddings::config::EmbeddingProvider;

    fn resolved(provider: EmbeddingProvider, base_url: Option<&str>) -> ResolvedEmbeddingProvider {
        ResolvedEmbeddingProvider {
            provider,
            api_key: "secret".to_string(),
            model: "text-embedding-3-small".to_string(),
            base_url: base_url.map(str::to_string),
            dimensions: 1536,
        }
    }

    /// The point of the whole module: one endpoint, one breaker, whatever the
    /// model, the width or the tenant's credential.
    #[test]
    fn one_endpoint_is_one_breaker_across_models_and_keys() {
        let mut a = resolved(EmbeddingProvider::OpenAI, Some("https://marvel.example/v1"));
        let mut b = resolved(
            EmbeddingProvider::OpenAI,
            Some("https://marvel.example/v1/"),
        );
        b.model = "bge-m3".to_string();
        b.dimensions = 1024;
        b.api_key = "another tenant's key".to_string();
        assert_eq!(upstream_key(&a), upstream_key(&b));

        a.base_url = Some("https://marvel.example/v1?x=1".to_string());
        assert_eq!(upstream_key(&a), upstream_key(&b));
    }

    #[test]
    fn different_hosts_and_protocols_are_different_breakers() {
        let a = resolved(EmbeddingProvider::OpenAI, Some("https://marvel.example/v1"));
        let b = resolved(EmbeddingProvider::OpenAI, Some("https://other.example/v1"));
        let c = resolved(EmbeddingProvider::Ollama, Some("https://marvel.example/v1"));
        assert_ne!(upstream_key(&a), upstream_key(&b));
        assert_ne!(upstream_key(&a), upstream_key(&c));
    }

    /// The vendor's own host is one shared failure domain, not an absence of
    /// one — every tenant on default OpenAI goes down together.
    #[test]
    fn the_vendor_default_is_a_shared_endpoint() {
        let a = resolved(EmbeddingProvider::OpenAI, None);
        let b = resolved(EmbeddingProvider::OpenAI, None);
        assert_eq!(upstream_key(&a), upstream_key(&b));
        assert_eq!(upstream_key(&a), "embedding:openai:vendor-default");
    }

    #[test]
    fn an_explicit_port_and_its_default_are_the_same_endpoint() {
        let a = resolved(EmbeddingProvider::Ollama, Some("http://localhost:11434"));
        let b = resolved(EmbeddingProvider::Ollama, Some("http://localhost:11434/v1"));
        assert_eq!(upstream_key(&a), upstream_key(&b));
    }
}
