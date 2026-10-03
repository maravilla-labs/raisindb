//! Reference resolution: the engine behind SQL `RESOLVE()`.
//!
//! A document (typically a node's `properties`) holds references —
//! `{"raisin:ref": <id or /path>, "raisin:workspace": ...}` — and RESOLVE
//! replaces each with the node it names, nesting up to a depth.
//!
//! There is ONE resolution path. There used to be five (`resolve`,
//! `resolve_inline`, `resolve_inline_with_depth`, `resolve_properties`,
//! `resolve_single_reference`); only the JSON one had a production caller, and
//! the others had drifted from it — none applied a locale, and the
//! `PropertyValue` ones refused to inline an id a second time.
//!
//! How a resolution runs:
//!
//! 1. **frontier** — a breadth-first walk over the TARGETS, not the document.
//!    Level 1 is the document's own references; level N+1 is only the
//!    references found in targets first reached at level N. Each target's
//!    outgoing references are collected ONCE, when it is fetched, with the one
//!    JSON walker ([`walk`]); a leaf ends the recursion without another walk.
//! 2. **fetch** — every target is decided BEFORE it is descended into: read at
//!    the statement's ONE snapshot, translated into the statement's locale, and
//!    passed through row-level security. Denied, missing and hidden-in-locale
//!    all yield `None`, which leaves the reference bare — the three are
//!    indistinguishable on purpose, so RESOLVE is no existence oracle.
//! 3. **memo** — per statement, keyed by `(workspace, locator)` plus the read
//!    scope `(snapshot, locale, fields)`. A target shared by every row of a
//!    listing is read once for the whole statement.
//! 4. **inline** — one substitution pass with depth tracking; a target is
//!    expanded once per `(target, remaining depth)` and cloned into every place
//!    it appears.
//! 5. **budget** — distinct targets, inlined occurrences and inlined bytes are
//!    bounded per statement, and exceeding a bound is an error naming RESOLVE,
//!    never a silently truncated document.

mod budget;
mod fetch;
mod frontier;
mod memo;
mod walk;

#[cfg(test)]
mod tests;

pub use budget::ResolveBudget;
pub use memo::{ResolveMemo, ResolveStats};

use crate::services::translation_resolver::TranslationResolver;
use memo::ReadScope;
use raisin_context::RepositoryConfig;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::Node;
use raisin_models::translations::LocaleCode;
use raisin_storage::Storage;
use std::sync::Arc;

/// Maximum depth for recursive reference resolution to prevent runaway queries.
pub const MAX_RESOLUTION_DEPTH: u32 = 10;

/// The language a resolution runs in.
///
/// Reference expansion used to be language-blind, which made it impossible to
/// read a translated document: `SELECT resolve(properties, 2) ... WHERE locale =
/// 'ja'` returned the SELECTED row translated — the scan executors apply the
/// overlay before projection — and every node it REFERENCES in the base
/// language. A reference is part of the document being read, so it is read in
/// the same language as the document. Absent (`None`) means the base language.
///
/// The overlay is read at the resolver's snapshot, the same revision the
/// targets are read at.
#[derive(Clone)]
pub struct ResolutionLocale {
    /// The locale to resolve referenced nodes in.
    pub locale: LocaleCode,
    /// The repository's default language — the language the base node IS.
    pub default_language: String,
    /// Fallback chains, so `fr-CA` can fall back to `fr` and then to the base.
    pub config: RepositoryConfig,
}

/// Resolves the references in a JSON document. Cheap to build: construct one
/// per evaluation and share the statement's state through [`Self::with_memo`]
/// and [`Self::with_translation_resolver`].
pub struct ReferenceResolver<S: Storage> {
    storage: Arc<S>,
    tenant_id: String,
    repo_id: String,
    branch: String,
    /// Every target is read at this revision — the statement's.
    snapshot: HLC,
    /// `None` = an internal caller with no identity, the convention every scan
    /// executor uses: no row-level security. A user is never `None`.
    auth: Option<AuthContext>,
    /// `None` = base language: fetch and inline exactly as stored.
    locale: Option<ResolutionLocale>,
    translations: Option<Arc<TranslationResolver<S::Translations>>>,
    memo: Arc<ResolveMemo>,
}

impl<S: Storage> ReferenceResolver<S> {
    /// A resolver reading targets at `snapshot`, with a private memo and the
    /// default budget. Pass the statement's memo with [`Self::with_memo`].
    pub fn new(
        storage: Arc<S>,
        tenant_id: impl Into<String>,
        repo_id: impl Into<String>,
        branch: impl Into<String>,
        snapshot: HLC,
    ) -> Self {
        Self {
            storage,
            tenant_id: tenant_id.into(),
            repo_id: repo_id.into(),
            branch: branch.into(),
            snapshot,
            auth: None,
            locale: None,
            translations: None,
            memo: Arc::new(ResolveMemo::default()),
        }
    }

    /// The identity every target is checked against. There is no opt-out: a
    /// caller with an identity always gets row-level security on targets.
    pub fn with_auth(mut self, auth: Option<AuthContext>) -> Self {
        self.auth = auth;
        self
    }

    /// Resolve referenced nodes in a locale; `None` for the base language.
    pub fn with_locale(mut self, locale: Option<ResolutionLocale>) -> Self {
        self.locale = locale;
        self
    }

    /// Reuse the statement's translation resolver instead of building one.
    pub fn with_translation_resolver(
        mut self,
        resolver: Arc<TranslationResolver<S::Translations>>,
    ) -> Self {
        self.translations = Some(resolver);
        self
    }

    /// Share a memo — and with it the budget — across every evaluation of one
    /// statement. NEVER share one across statements: it holds results already
    /// filtered for one caller at one snapshot.
    pub fn with_memo(mut self, memo: Arc<ResolveMemo>) -> Self {
        self.memo = memo;
        self
    }

    /// Resolve every reference inside a JSON value.
    ///
    /// - a reference is any object carrying a string `raisin:ref` (an id, or a
    ///   path when it starts with `/`); its `raisin:workspace` defaults to
    ///   `workspace` when absent or empty — also for references found inside
    ///   inlined targets;
    /// - `max_depth` (capped at [`MAX_RESOLUTION_DEPTH`]) bounds how deep
    ///   inlined nodes nest, which is also what makes a cycle terminate;
    /// - a reference that cannot be resolved — missing, hidden in the locale,
    ///   or not readable by the caller — is kept as it was written;
    /// - `fields`, when given, trims every inlined node to `id`, `name`,
    ///   `path`, `node_type` plus the listed properties, and only references
    ///   inside what is kept are followed.
    ///
    /// The value may itself be a single reference, which resolves to the node.
    pub async fn resolve_json(
        &self,
        workspace: &str,
        value: &serde_json::Value,
        max_depth: u32,
        fields: Option<&[String]>,
    ) -> Result<serde_json::Value> {
        let depth = max_depth.min(MAX_RESOLUTION_DEPTH);
        if depth == 0 {
            return Ok(value.clone());
        }

        let read = Arc::new(ReadScope {
            snapshot: self.snapshot,
            locale: self
                .effective_locale()
                .map(|l| l.locale.as_str().to_string()),
            fields: fields.map(<[String]>::to_vec),
        });
        let resolved = self.gather(workspace, value, depth, &read, fields).await?;

        let mut out = value.clone();
        if resolved.values().all(Option::is_none) {
            return Ok(out);
        }
        let totals = walk::Inliner::new(
            workspace,
            &resolved,
            self.memo.allowance(),
            self.memo.budget(),
        )
        .inline(&mut out, depth)?;
        self.memo.charge(totals)?;
        Ok(out)
    }

    /// The locale to translate targets into, or `None` when that is the base
    /// language anyway.
    fn effective_locale(&self) -> Option<&ResolutionLocale> {
        self.locale
            .as_ref()
            .filter(|l| l.locale.as_str() != l.default_language)
    }
}

/// Convert a Node to a `serde_json::Value` for RESOLVE() SQL function output
///
/// Returns an object with: id, name, path, node_type, plus all properties flattened.
pub fn node_to_json_value(node: &Node) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    map.insert("id".to_string(), serde_json::Value::String(node.id.clone()));
    map.insert(
        "name".to_string(),
        serde_json::Value::String(node.name.clone()),
    );
    map.insert(
        "path".to_string(),
        serde_json::Value::String(node.path.clone()),
    );
    map.insert(
        "node_type".to_string(),
        serde_json::Value::String(node.node_type.clone()),
    );

    // Flatten properties into the object
    if let Ok(serde_json::Value::Object(props_map)) = serde_json::to_value(&node.properties) {
        for (k, v) in props_map {
            map.insert(k, v);
        }
    }

    serde_json::Value::Object(map)
}

/// [`node_to_json_value`], optionally keeping only `fields` of the properties.
///
/// The identity members (`id`, `name`, `path`, `node_type`) are always kept:
/// they are what a renderer links and keys by, and a trimmed node without them
/// could not be told apart from its neighbours.
pub fn node_to_json_value_with_fields(node: &Node, fields: Option<&[String]>) -> serde_json::Value {
    let Some(fields) = fields else {
        return node_to_json_value(node);
    };
    let mut map = serde_json::Map::with_capacity(4 + fields.len());
    map.insert("id".into(), serde_json::Value::String(node.id.clone()));
    map.insert("name".into(), serde_json::Value::String(node.name.clone()));
    map.insert("path".into(), serde_json::Value::String(node.path.clone()));
    map.insert(
        "node_type".into(),
        serde_json::Value::String(node.node_type.clone()),
    );
    for field in fields {
        if let Some(value) = node.properties.get(field) {
            if let Ok(json) = serde_json::to_value(value) {
                map.insert(field.clone(), json);
            }
        }
    }
    serde_json::Value::Object(map)
}
