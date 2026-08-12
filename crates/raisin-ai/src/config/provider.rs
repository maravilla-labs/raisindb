//! AI provider types, model configuration, and use cases.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// AI use cases supported by the system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AIUseCase {
    /// Text embeddings for semantic search
    Embedding,
    /// Chat/conversational AI
    Chat,
    /// Agentic workflows with tool use
    Agent,
    /// Text completion/generation
    Completion,
    /// Text classification tasks
    Classification,
}

/// Supported AI providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AIProvider {
    /// OpenAI (GPT models)
    #[serde(rename = "openai")]
    OpenAI,
    /// Anthropic (Claude models)
    #[serde(rename = "anthropic")]
    Anthropic,
    /// Google (Gemini, PaLM)
    #[serde(rename = "google")]
    Google,
    /// Ollama (local models via API)
    #[serde(rename = "ollama")]
    Ollama,
    /// Azure OpenAI Service
    #[serde(rename = "azure_openai")]
    AzureOpenAI,
    /// Groq (fast inference for open-source models)
    #[serde(rename = "groq")]
    Groq,
    /// OpenRouter (multi-provider router with unified API)
    #[serde(rename = "openrouter")]
    OpenRouter,
    /// AWS Bedrock (Claude, Nova, Llama models via AWS)
    #[serde(rename = "bedrock")]
    Bedrock,
    /// Custom provider
    #[serde(rename = "custom")]
    Custom,
    /// Local Candle models (Moondream, BLIP, CLIP) - runs in-process
    #[serde(rename = "local")]
    Local,
}

impl AIProvider {
    /// Returns the serde serialization name for this provider.
    ///
    /// Used for matching model prefixes like `openai:gpt-4o`.
    pub fn serde_name(&self) -> &'static str {
        match self {
            AIProvider::OpenAI => "openai",
            AIProvider::Anthropic => "anthropic",
            AIProvider::Google => "google",
            AIProvider::Ollama => "ollama",
            AIProvider::AzureOpenAI => "azure_openai",
            AIProvider::Groq => "groq",
            AIProvider::OpenRouter => "openrouter",
            AIProvider::Bedrock => "bedrock",
            AIProvider::Custom => "custom",
            AIProvider::Local => "local",
        }
    }

    /// Parse a provider from its serde name.
    ///
    /// Returns `None` if the name doesn't match any known provider.
    pub fn from_serde_name(name: &str) -> Option<Self> {
        match name {
            "openai" => Some(AIProvider::OpenAI),
            "anthropic" => Some(AIProvider::Anthropic),
            "google" => Some(AIProvider::Google),
            "ollama" => Some(AIProvider::Ollama),
            "azure_openai" => Some(AIProvider::AzureOpenAI),
            "groq" => Some(AIProvider::Groq),
            "openrouter" => Some(AIProvider::OpenRouter),
            "bedrock" => Some(AIProvider::Bedrock),
            "custom" => Some(AIProvider::Custom),
            "local" => Some(AIProvider::Local),
            _ => None,
        }
    }

    /// Returns the default API endpoint for this provider.
    ///
    /// Returns `None` for providers that require custom endpoints or run locally.
    pub fn default_endpoint(&self) -> Option<&'static str> {
        match self {
            AIProvider::OpenAI => Some("https://api.openai.com/v1"),
            AIProvider::Anthropic => Some("https://api.anthropic.com/v1"),
            AIProvider::Google => Some("https://generativelanguage.googleapis.com/v1"),
            AIProvider::Ollama => Some("http://localhost:11434"),
            AIProvider::Groq => Some("https://api.groq.com/openai/v1"),
            AIProvider::OpenRouter => Some("https://openrouter.ai/api/v1"),
            AIProvider::AzureOpenAI => None, // Requires tenant-specific endpoint
            AIProvider::Bedrock => None,     // Requires AWS region-specific endpoint
            AIProvider::Custom => None,      // Requires custom endpoint
            AIProvider::Local => None,       // Runs in-process, no endpoint needed
        }
    }

    /// Returns whether this provider requires an API key.
    pub fn requires_api_key(&self) -> bool {
        match self {
            AIProvider::OpenAI
            | AIProvider::Anthropic
            | AIProvider::Google
            | AIProvider::AzureOpenAI
            | AIProvider::Groq
            | AIProvider::OpenRouter
            | AIProvider::Bedrock => true,
            AIProvider::Ollama | AIProvider::Custom | AIProvider::Local => false,
        }
    }

    /// Returns whether this provider runs in-process (no network call).
    pub fn is_local(&self) -> bool {
        matches!(self, AIProvider::Local)
    }
}

/// Maximum length of a provider slug (see [`validate_slug`]).
pub const MAX_SLUG_LEN: usize = 39;

/// Reasons a provider slug can be rejected.
///
/// Slugs are user-chosen and immutable once created, so every rejection has to
/// happen at write time — nothing downstream re-validates them.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SlugError {
    #[error("provider slug must not be empty")]
    Empty,

    #[error("provider slug must be at most {MAX_SLUG_LEN} characters, got {0}")]
    TooLong(usize),

    #[error("provider slug must start with a lowercase letter or digit")]
    BadFirstChar,

    #[error("provider slug may only contain lowercase letters, digits and '-', found '{0}'")]
    BadChar(char),

    #[error(
        "provider slug '{slug}' is the legacy default slug of the '{slug}' provider kind \
         and cannot name a '{kind}' entry"
    )]
    ForeignKindName {
        /// The offending slug — always some other kind's serde name.
        slug: String,
        /// The serde name of the kind the entry actually is.
        kind: &'static str,
    },
}

/// Validates a per-tenant provider slug against `^[a-z0-9][a-z0-9-]{0,38}$`,
/// with an entry's OWN kind name grandfathered in.
///
/// The slug is the prefix half of a model id (`<slug>:<model>`), which is why
/// `:` — and everything else outside the character class — has to be rejected
/// here: a slug containing `:` would make `parse_model_id` split in the wrong
/// place and silently resolve to a different provider, or to none at all.
///
/// A kind's own name is exempt from the pattern because it *is* the legacy
/// default slug for that kind: an entry written before slugs existed is
/// addressed by its kind's serde name, and `azure_openai` fails the pattern on
/// its underscore. Clients that predate slugs send those names on every save,
/// including for entries they have never created, so exempting them only on
/// update is not enough — a tenant's first Azure save would 400 with nothing it
/// could do about it. Hence the carve-out lives in the predicate, where every
/// caller inherits it.
///
/// It is a carve-out for *that* kind only, which is why this takes the entry's
/// kind. A slug that is some other kind's name is refused outright rather than
/// merely held to the pattern: `openai` passes the pattern happily, so without
/// this an Anthropic entry could be created under slug `openai` and every legacy
/// `openai:gpt-4o` id in the tenant's data would silently start resolving to
/// Anthropic — permanently, since slugs are immutable.
pub fn validate_slug(slug: &str, kind: AIProvider) -> std::result::Result<(), SlugError> {
    match AIProvider::from_serde_name(slug) {
        // Exactly this entry's kind name: the legacy default slug for it.
        Some(named) if named == kind => return Ok(()),
        Some(_) => {
            return Err(SlugError::ForeignKindName {
                slug: slug.to_string(),
                kind: kind.serde_name(),
            })
        }
        None => {}
    }

    if slug.is_empty() {
        return Err(SlugError::Empty);
    }
    if slug.len() > MAX_SLUG_LEN {
        return Err(SlugError::TooLong(slug.len()));
    }

    let mut chars = slug.chars();
    // Unwrap is safe: emptiness was rejected above.
    let first = chars.next().unwrap();
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return Err(SlugError::BadFirstChar);
    }
    for c in chars {
        if !c.is_ascii_lowercase() && !c.is_ascii_digit() && c != '-' {
            return Err(SlugError::BadChar(c));
        }
    }
    Ok(())
}

/// Configuration for a single AI provider.
///
/// A provider is identified by its per-tenant [`slug`](Self::slug), not by its
/// [`kind`](Self::kind): a tenant can configure two OpenAI-compatible gateways
/// side by side as long as their slugs differ.
///
/// Deserialization goes through `StoredAIProviderConfig` so that entries
/// written before slugs existed get `slug = kind.serde_name()`. That default
/// cannot be expressed with `#[serde(default)]` — a field default can't see its
/// siblings — and doing it in the RocksDB repository instead would leave every
/// other store (JSON request bodies, test fixtures, exports) without a slug.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(from = "StoredAIProviderConfig")]
pub struct AIProviderConfig {
    /// Per-tenant unique identifier, chosen by the user and immutable once
    /// created. Doubles as the model-id prefix (`<slug>:<model>`).
    pub slug: String,
    /// The AI provider type — which wire protocol/SDK to talk.
    ///
    /// Keeps the wire name `provider`: renaming the JSON/MessagePack key would
    /// make every stored provider type deserialize as missing.
    #[serde(rename = "provider")]
    pub kind: AIProvider,
    /// Human-readable name shown in the UI (defaults to the slug when absent)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Optional logo URL shown next to the display name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
    /// Encrypted API key (using AES-256-GCM)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key_encrypted: Option<Vec<u8>>,
    /// Optional custom API endpoint (for self-hosted or custom providers)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_endpoint: Option<String>,
    /// Whether this provider is enabled
    pub enabled: bool,
    /// List of models available from this provider
    pub models: Vec<AIModelConfig>,
}

/// On-the-wire shape of [`AIProviderConfig`], with `slug` optional.
///
/// Only exists to carry the back-compat default; never construct it directly.
#[derive(Deserialize)]
struct StoredAIProviderConfig {
    #[serde(default)]
    slug: Option<String>,
    #[serde(rename = "provider")]
    kind: AIProvider,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    icon_url: Option<String>,
    #[serde(default)]
    api_key_encrypted: Option<Vec<u8>>,
    #[serde(default)]
    api_endpoint: Option<String>,
    enabled: bool,
    #[serde(default)]
    models: Vec<AIModelConfig>,
}

impl From<StoredAIProviderConfig> for AIProviderConfig {
    fn from(stored: StoredAIProviderConfig) -> Self {
        Self {
            // Legacy entries are addressed by the enum's serde name, which is
            // exactly what old model ids (`openai:gpt-4o`, `custom:x/y`) and
            // the existing route URLs use — so adopting it as the slug keeps
            // every stored reference resolving unchanged.
            slug: stored
                .slug
                .unwrap_or_else(|| stored.kind.serde_name().to_string()),
            kind: stored.kind,
            display_name: stored.display_name,
            icon_url: stored.icon_url,
            api_key_encrypted: stored.api_key_encrypted,
            api_endpoint: stored.api_endpoint,
            enabled: stored.enabled,
            models: stored.models,
        }
    }
}

/// Makes every slug in a freshly-read provider list unique, renaming collisions
/// to `<slug>-2`, `<slug>-3`, … in list order.
///
/// Two entries can share a slug only through the back-compat default above: a
/// config written before slugs existed may hold two entries of one kind — a
/// hosted gateway and a tenant's own box, both `custom`, which is the exact
/// collision slugs were introduced to fix — and both read back as `custom`.
///
/// Renaming is chosen over the two alternatives deliberately:
///
/// - *Leaving the duplicates alone* is the one unacceptable option. Everything
///   downstream addresses entries by slug and stops at the first match, so a PUT
///   would update entry one while `DELETE /providers/custom` removed both, and
///   the second entry would be unreachable but still live.
/// - *Dropping* the later duplicates loses a configured endpoint and its
///   encrypted API key on a read, which is precisely the silent data loss the
///   merge semantics exist to prevent.
/// - *Rejecting* the config loudly would 500 every AI request for a tenant whose
///   data is merely old, with no way for them to fix it — the repair path itself
///   goes through this read.
///
/// Renaming keeps both entries, keeps them addressable, and is deterministic:
/// the first entry keeps the bare kind name, so every stored `custom:<model>` id
/// resolves exactly where it resolved before. The suffixed entry has to be
/// re-pointed at by hand, which is visible in the UI rather than silent.
pub(super) fn dedupe_slugs(mut providers: Vec<AIProviderConfig>) -> Vec<AIProviderConfig> {
    // Every slug in the list, so a rename can never collide with an entry that
    // has not been reached yet.
    let mut taken: std::collections::HashSet<String> =
        providers.iter().map(|p| p.slug.clone()).collect();
    if taken.len() == providers.len() {
        return providers; // The overwhelmingly common case: nothing to do.
    }

    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for provider in providers.iter_mut() {
        if seen.insert(provider.slug.clone()) {
            continue;
        }
        let renamed = (2..)
            .map(|n| format!("{}-{}", provider.slug, n))
            .find(|candidate| !taken.contains(candidate))
            // Unreachable: the candidate space is unbounded.
            .expect("a free slug suffix always exists");
        tracing::warn!(
            duplicate_slug = %provider.slug,
            renamed_to = %renamed,
            "Provider config held two entries under one slug (a pre-slug config with two \
             entries of one kind); the later one was renamed so it stays addressable"
        );
        taken.insert(renamed.clone());
        seen.insert(renamed.clone());
        provider.slug = renamed;
    }

    providers
}

impl AIProviderConfig {
    /// Creates a new provider configuration slugged after the provider kind.
    ///
    /// This is the pre-slug behaviour: one entry per kind, addressed by the
    /// kind's serde name. Use [`with_slug`](Self::with_slug) for anything a
    /// tenant names itself.
    pub fn new(kind: AIProvider) -> Self {
        Self::with_slug(kind.serde_name(), kind)
    }

    /// Creates a new provider configuration under an explicit slug.
    ///
    /// The slug is not validated here — [`validate_slug`] is the API-boundary
    /// check, and internal callers construct slugs that are known-good.
    pub fn with_slug(slug: impl Into<String>, kind: AIProvider) -> Self {
        Self {
            slug: slug.into(),
            kind,
            display_name: None,
            icon_url: None,
            api_key_encrypted: None,
            api_endpoint: None,
            enabled: true,
            models: Vec::new(),
        }
    }

    /// Gets the default model for a specific use case.
    pub fn get_default_model(&self, use_case: AIUseCase) -> Option<&AIModelConfig> {
        self.models
            .iter()
            .find(|m| m.use_cases.contains(&use_case) && m.is_default)
    }
}

/// Configuration for a specific AI model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AIModelConfig {
    /// Unique identifier for the model (e.g., "gpt-4", "claude-3-opus")
    pub model_id: String,
    /// Human-readable display name
    pub display_name: String,
    /// Use cases this model supports
    pub use_cases: Vec<AIUseCase>,
    /// Default temperature for sampling (0.0-2.0)
    pub default_temperature: f32,
    /// Default maximum tokens to generate
    pub default_max_tokens: u32,
    /// Whether this is the default model for its use cases
    pub is_default: bool,
    /// Optional metadata (architecture, embedding_length, etc. from provider)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

impl AIModelConfig {
    /// Creates a new model configuration.
    pub fn new(model_id: String, display_name: String) -> Self {
        Self {
            model_id,
            display_name,
            use_cases: Vec::new(),
            default_temperature: 0.7,
            default_max_tokens: 1024,
            is_default: false,
            metadata: None,
        }
    }

    /// Creates a new model configuration with metadata.
    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = Some(metadata);
        self
    }
}
