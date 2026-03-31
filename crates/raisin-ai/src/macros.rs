//! Macros for AI provider implementations.
//!
//! This module provides macros to reduce boilerplate in AI provider implementations.

/// Macro to generate standard AI provider struct and constructors.
///
/// This macro generates:
/// - A provider struct with `api_key`, `client`, `base_url`, and `cache` fields
/// - A `new()` constructor that takes an API key and uses the default base URL
/// - A `with_base_url()` constructor that takes both API key and custom base URL
///
/// # Example
///
/// ```ignore
/// use crate::impl_ai_provider;
///
/// const MY_API_BASE: &str = "https://api.example.com/v1";
/// const MODEL_CACHE_TTL: Duration = Duration::from_secs(3600);
///
/// impl_ai_provider!(
///     /// My AI provider for example.com
///     MyProvider,
///     MY_API_BASE,
///     MODEL_CACHE_TTL
/// );
/// ```
///
/// This generates:
///
/// ```ignore
/// #[derive(Debug, Clone)]
/// pub struct MyProvider {
///     api_key: String,
///     client: Client,
///     base_url: String,
///     cache: Arc<ModelCache>,
/// }
///
/// impl MyProvider {
///     pub fn new(api_key: impl Into<String>) -> Self {
///         Self {
///             api_key: api_key.into(),
///             client: Client::new(),
///             base_url: MY_API_BASE.to_string(),
///             cache: Arc::new(ModelCache::with_ttl(MODEL_CACHE_TTL)),
///         }
///     }
///
///     pub fn with_base_url(api_key: impl Into<String>, base_url: impl Into<String>) -> Self {
///         Self {
///             api_key: api_key.into(),
///             client: Client::new(),
///             base_url: base_url.into(),
///             cache: Arc::new(ModelCache::with_ttl(MODEL_CACHE_TTL)),
///         }
///     }
/// }
/// ```
#[macro_export]
macro_rules! impl_ai_provider {
    (
        $(#[$meta:meta])*
        $name:ident,
        $default_base_url:expr,
        $cache_ttl:expr
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone)]
        pub struct $name {
            api_key: String,
            client: reqwest::Client,
            base_url: String,
            cache: std::sync::Arc<$crate::model_cache::ModelCache>,
        }

        impl $name {
            /// Creates a new provider with the given API key using the default base URL
            pub fn new(api_key: impl Into<String>) -> Self {
                Self {
                    api_key: api_key.into(),
                    client: reqwest::Client::new(),
                    base_url: $default_base_url.to_string(),
                    cache: std::sync::Arc::new($crate::model_cache::ModelCache::with_ttl($cache_ttl)),
                }
            }

            /// Creates a new provider with custom base URL
            pub fn with_base_url(api_key: impl Into<String>, base_url: impl Into<String>) -> Self {
                Self {
                    api_key: api_key.into(),
                    client: reqwest::Client::new(),
                    base_url: base_url.into(),
                    cache: std::sync::Arc::new($crate::model_cache::ModelCache::with_ttl($cache_ttl)),
                }
            }
        }
    };
}

pub use impl_ai_provider;
