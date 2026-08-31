// SPDX-License-Identifier: BSL-1.1

//! The single classifier that turns a discovered [`ModelInfo`] into the
//! `use_cases` + `metadata` an `AIModelConfig` is built from.
//!
//! There used to be two of these — one inlined in the `/ai/models` refresh
//! handler and one in the `/providers/{slug}/test` handler — and they only
//! nearly agreed. Two copies of a fail-closed rule is a fail-closed rule that
//! fails open on one surface, so both now call [`classify`].
//!
//! # The wire contract it consumes
//!
//! An OpenAI-shaped gateway publishes two extension keys on each entry of
//! `GET /v1/models`: `kind` (`"chat"` | `"embedding"`) and, iff the kind is
//! `embedding`, `dimensions`. Provider clients fold those into [`ModelInfo`]
//! via [`apply_declared_kind`]; this module reads them back out of `metadata`
//! and decides what the model may be offered for.
//!
//! # Why a missing width must fail CLOSED
//!
//! The width is hashed into `EmbedderId` (`provider:model:dimensions:tokenizer`,
//! `config/embedder.rs`), which becomes the `{embedder_hash}` segment of every
//! `cf::EMBEDDINGS` key and the partition the vector index is built under. A
//! guessed width does not error — it produces a correctly-shaped index whose
//! vectors are ranked against the wrong neighbours, forever, with no message
//! anywhere. Fixing it means re-embedding every tenant on that alias.
//!
//! So: an embedding model with no published width keeps NO embedding use case,
//! gets NO `embedding_length` (not `0`, not `null`, not a guess), and carries a
//! reason string instead. It stays in the list — dropping it would make the
//! failure undiagnosable, since the operator would just see a model that does
//! not exist.

use serde_json::Value;

use crate::config::AIUseCase;
use crate::model_cache::{ModelCapabilities, ModelInfo};

/// `metadata` key echoing the gateway's declared kind. **Diagnostic only** —
/// nothing may branch on it. Capability lives in `use_cases`.
pub const METADATA_KIND: &str = "kind";

/// `metadata` key carrying the embedding width. The established key, already
/// written by the OpenAI and Ollama discovery paths and already read by the
/// admin console. Do not invent a second one for the same number.
pub const METADATA_EMBEDDING_LENGTH: &str = "embedding_length";

/// `metadata` key explaining why an apparently-embedding model is not offered.
pub const METADATA_EMBEDDING_UNAVAILABLE_REASON: &str = "embedding_unavailable_reason";

/// The exact reason string written when the width is missing or unusable.
pub const EMBEDDING_UNAVAILABLE_REASON: &str = "dimensions missing or invalid";

/// Upper bound on a plausible embedding width. Anything outside `1..=65536` is
/// treated as absent rather than trusted.
pub const MAX_EMBEDDING_DIMENSIONS: u64 = 65_536;

/// What the gateway said this model is.
///
/// `kind` is a CLOSED set to writers and an OPEN set to readers: a future
/// `"rerank"` must not error, panic, or drop the listing — it lands in
/// [`DeclaredKind::Unknown`] and the per-provider heuristics stand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclaredKind {
    /// No `kind` key, or one whose JSON type is not a string.
    Absent,
    /// `"chat"`.
    Chat,
    /// `"embedding"`.
    Embedding,
    /// A string we do not recognise.
    Unknown,
}

impl DeclaredKind {
    /// True when the gateway declared something we understood.
    ///
    /// The "default to chat when nothing was detected" fallback is skipped in
    /// that case: offering a declared embedding model for chat 400s at the
    /// gateway.
    fn is_explicit(self) -> bool {
        matches!(self, DeclaredKind::Chat | DeclaredKind::Embedding)
    }
}

/// Who is being classified, for the log line. Two `&str` that mean different
/// things travel as named fields so they cannot be swapped at a call site.
#[derive(Debug, Clone, Copy, Default)]
pub struct ClassificationContext<'a> {
    /// Tenant whose config is being refreshed.
    pub tenant_id: Option<&'a str>,
    /// Provider ENTRY slug (not its kind) — a tenant may have two gateways.
    pub provider_slug: Option<&'a str>,
}

/// The classifier's output: what the model may be used for, and the metadata
/// to persist alongside it.
#[derive(Debug, Clone)]
pub struct Classification {
    /// Use cases for `AIModelConfig::use_cases`.
    pub use_cases: Vec<AIUseCase>,
    /// Metadata for `AIModelConfig::metadata`, annotated where needed.
    pub metadata: Option<Value>,
}

/// Reads the declared kind back out of a model's metadata.
pub fn declared_kind(metadata: Option<&Value>) -> DeclaredKind {
    match metadata
        .and_then(|m| m.get(METADATA_KIND))
        .and_then(Value::as_str)
    {
        None => DeclaredKind::Absent,
        Some("chat") => DeclaredKind::Chat,
        Some("embedding") => DeclaredKind::Embedding,
        Some(_) => DeclaredKind::Unknown,
    }
}

/// The published embedding width, or `None` if absent or implausible.
///
/// An unexpected JSON *type* is an absence, never an error: one malformed entry
/// must not fail the listing for every other model beside it.
pub fn embedding_width(metadata: Option<&Value>) -> Option<u64> {
    let raw = metadata.and_then(|m| m.get(METADATA_EMBEDDING_LENGTH))?;
    let d = raw.as_u64()?;
    is_plausible_width(d).then_some(d)
}

fn is_plausible_width(d: u64) -> bool {
    (1..=MAX_EMBEDDING_DIMENSIONS).contains(&d)
}

/// Folds a gateway's declared `kind`/`dimensions` into a provider client's
/// capabilities and metadata, before the model leaves the client.
///
/// A `None` kind leaves both untouched, so a provider that publishes neither
/// (real Groq, and every gateway response predating the contract) behaves
/// byte-for-byte as it did.
pub fn apply_declared_kind(
    model_id: &str,
    kind: Option<&str>,
    dimensions: Option<u64>,
    capabilities: &mut ModelCapabilities,
    metadata: &mut Value,
) {
    let Some(kind) = kind else {
        return;
    };
    metadata[METADATA_KIND] = Value::from(kind);

    match kind {
        "embedding" => {
            let width = dimensions.filter(|d| is_plausible_width(*d));
            // An embedding model is not a chat model: leaving `chat` true would
            // let the picker offer it for completions.
            capabilities.chat = false;
            capabilities.tools = false;
            capabilities.streaming = false;
            capabilities.vision = false;
            capabilities.embeddings = width.is_some();
            match width {
                Some(d) => metadata[METADATA_EMBEDDING_LENGTH] = Value::from(d),
                // Not 0, not null, not a guess — the key must not exist.
                None => remove_key(metadata, METADATA_EMBEDDING_LENGTH),
            }
        }
        "chat" => {
            capabilities.chat = true;
            capabilities.embeddings = false;
            remove_key(metadata, METADATA_EMBEDDING_LENGTH);
            if dimensions.is_some() {
                tracing::warn!(
                    model_id,
                    "gateway published a chat model carrying `dimensions`; ignoring the width"
                );
            }
        }
        // Unknown to us, known to someone: keep the echo, keep the heuristics.
        _ => {}
    }
}

/// Classifies one discovered model. The ONLY place capability is decided.
pub fn classify(model: &ModelInfo, ctx: ClassificationContext<'_>) -> Classification {
    let kind = declared_kind(model.metadata.as_ref());
    let width = embedding_width(model.metadata.as_ref());
    let caps = &model.capabilities;

    // A model claims to be an embedder either because the gateway said so or
    // because the provider client's heuristics decided so. Both must be read:
    // `apply_declared_kind` has already cleared `capabilities.embeddings` for a
    // declared embedder with no usable width, so keying off capabilities alone
    // would leave that model silently unexplained.
    let claims_embeddings = caps.embeddings || kind == DeclaredKind::Embedding;
    // Claiming embeddings without a published width is the one-way door.
    let embeddings_offered = claims_embeddings && width.is_some();
    let width_missing = claims_embeddings && width.is_none();

    let mut use_cases = Vec::new();
    if caps.chat {
        use_cases.push(AIUseCase::Chat);
        use_cases.push(AIUseCase::Completion);
    }
    if embeddings_offered {
        use_cases.push(AIUseCase::Embedding);
    }
    if caps.tools {
        use_cases.push(AIUseCase::Agent);
    }

    // Legacy fallback: a provider that detected nothing at all used to yield a
    // chat model. Keep that — but never for a model that declared a kind, and
    // never for one claiming embeddings, or a withheld embedder would come back
    // as a chat model and 400 at the gateway.
    if use_cases.is_empty() && !kind.is_explicit() && !claims_embeddings {
        use_cases.push(AIUseCase::Chat);
    }

    let mut metadata = model.metadata.clone();
    if width_missing {
        let meta = metadata.get_or_insert_with(|| Value::Object(Default::default()));
        // A width we refused to trust must not survive in the stored metadata:
        // the console reads this exact key to auto-set `dimensions`.
        remove_key(meta, METADATA_EMBEDDING_LENGTH);
        meta[METADATA_EMBEDDING_UNAVAILABLE_REASON] = Value::from(EMBEDDING_UNAVAILABLE_REASON);
        tracing::warn!(
            tenant_id = ctx.tenant_id.unwrap_or("-"),
            provider_slug = ctx.provider_slug.unwrap_or("-"),
            model_id = %model.id,
            "model claims embeddings but published no usable width; not offering it as an \
             embedder (a guessed width is hashed into the embedder identity and cannot be \
             corrected without re-embedding every tenant on it)"
        );
    }

    Classification {
        use_cases,
        metadata,
    }
}

fn remove_key(value: &mut Value, key: &str) {
    if let Some(obj) = value.as_object_mut() {
        obj.remove(key);
    }
}

#[cfg(test)]
mod tests;
