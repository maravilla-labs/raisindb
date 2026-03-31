//! Repository configuration and information types.

use serde::{Deserialize, Serialize};

/// Configuration for a repository
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RepositoryConfig {
    /// Default branch for workspaces (e.g., "main", "production")
    pub default_branch: String,

    /// Human-readable description
    pub description: Option<String>,

    /// Custom tags/metadata
    #[serde(default)]
    pub tags: std::collections::HashMap<String, String>,

    /// Default language for content (IMMUTABLE after repository creation)
    /// This is the primary language for the repository and cannot be changed
    /// after the repository is created to ensure indexing consistency.
    #[serde(default = "default_language")]
    pub default_language: String,

    /// List of supported languages for translations
    /// Must always include the default language
    #[serde(default = "default_supported_languages")]
    pub supported_languages: Vec<String>,

    /// Locale fallback chains for translation resolution.
    ///
    /// Maps a locale to its fallback sequence. When resolving a translation
    /// for a locale, the system tries each locale in the chain until a
    /// translation is found or the chain is exhausted.
    ///
    /// # Example
    ///
    /// ```json
    /// {
    ///   "fr-CA": ["fr", "en"],
    ///   "de-CH": ["de", "en"],
    ///   "es-MX": ["es", "en"]
    /// }
    /// ```
    ///
    /// With this configuration, a request for `fr-CA` will try:
    /// 1. fr-CA translation
    /// 2. fr translation (first fallback)
    /// 3. en translation (second fallback)
    /// 4. Base node (no translation)
    ///
    /// # Validation Rules
    ///
    /// - All locales (keys and values) must exist in `supported_languages`
    /// - Chains should ultimately resolve to `default_language`
    /// - Circular references are not allowed
    #[serde(default)]
    pub locale_fallback_chains: std::collections::HashMap<String, Vec<String>>,
}

fn default_language() -> String {
    "en".to_string()
}

fn default_supported_languages() -> Vec<String> {
    vec!["en".to_string()]
}

impl RepositoryConfig {
    /// Validate the locale fallback chains configuration.
    ///
    /// Ensures that:
    /// 1. All locale codes (keys and values) exist in `supported_languages`
    /// 2. No circular references exist in fallback chains
    /// 3. `default_language` is in `supported_languages`
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` if valid, or an error describing the validation failure.
    pub fn validate_locale_fallback_chains(&self) -> Result<(), String> {
        // Check that default_language is in supported_languages
        if !self.supported_languages.contains(&self.default_language) {
            return Err(format!(
                "default_language '{}' must be in supported_languages",
                self.default_language
            ));
        }

        // Check that all locale codes in chains exist in supported_languages
        for (locale, fallbacks) in &self.locale_fallback_chains {
            // Check the key (locale)
            if !self.supported_languages.contains(locale) {
                return Err(format!(
                    "Locale '{}' in fallback chain not found in supported_languages",
                    locale
                ));
            }

            // Check all fallback locales
            for fallback in fallbacks {
                if !self.supported_languages.contains(fallback) {
                    return Err(format!(
                        "Fallback locale '{}' for '{}' not found in supported_languages",
                        fallback, locale
                    ));
                }
            }

            // Check for circular references (simple self-reference check)
            if fallbacks.contains(locale) {
                return Err(format!(
                    "Circular reference detected: '{}' references itself in fallback chain",
                    locale
                ));
            }
        }

        Ok(())
    }

    /// Get the fallback chain for a locale.
    ///
    /// Returns the configured fallback chain, or a default chain if none is configured:
    /// - If locale has region (e.g., "fr-CA"), default chain is [language_only, default_language]
    /// - Otherwise, default chain is [default_language]
    ///
    /// # Example
    ///
    /// ```
    /// use raisin_context::RepositoryConfig;
    ///
    /// let config = RepositoryConfig::default();
    /// let chain = config.get_fallback_chain("fr-CA");
    /// // Returns ["fr-CA", "fr", "en"] (locale, language parent, then default)
    /// ```
    pub fn get_fallback_chain(&self, locale: &str) -> Vec<String> {
        // Start with the requested locale itself
        let mut chain = vec![locale.to_string()];

        // If explicitly configured, append configured fallbacks
        if let Some(configured_chain) = self.locale_fallback_chains.get(locale) {
            for fallback in configured_chain {
                if !chain.contains(fallback) {
                    chain.push(fallback.clone());
                }
            }
            return chain;
        }

        // Otherwise, generate automatic fallback chain
        // If locale has a region (e.g., "fr-CA"), fall back to language only (e.g., "fr")
        if let Some(hyphen_pos) = locale.find('-') {
            let language_only = &locale[..hyphen_pos];
            if language_only != locale && !chain.contains(&language_only.to_string()) {
                chain.push(language_only.to_string());
            }
        }

        // Always fall back to default language (unless already present)
        if !chain.contains(&self.default_language) && locale != self.default_language {
            chain.push(self.default_language.clone());
        }

        chain
    }
}

impl Default for RepositoryConfig {
    fn default() -> Self {
        Self {
            default_branch: "main".to_string(),
            description: None,
            tags: std::collections::HashMap::new(),
            default_language: default_language(),
            supported_languages: default_supported_languages(),
            locale_fallback_chains: std::collections::HashMap::new(),
        }
    }
}

/// Information about a repository
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RepositoryInfo {
    /// Tenant ID
    pub tenant_id: String,

    /// Repository ID
    pub repo_id: String,

    /// When the repository was created
    pub created_at: chrono::DateTime<chrono::Utc>,

    /// Available branches
    pub branches: Vec<String>,

    /// Repository configuration
    pub config: RepositoryConfig,
}
