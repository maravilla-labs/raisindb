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
mod doc;
mod fetch;
mod frontier;
mod json_api;
mod json_len;
mod memo;
mod walk;

#[cfg(test)]
mod tests;

pub use budget::ResolveBudget;
pub use json_api::{document_from_json, node_to_json_value, node_to_json_value_with_fields};
pub use memo::{ResolveMemo, ResolveStats};

use crate::services::translation_resolver::TranslationResolver;
use memo::ReadScope;
use raisin_context::RepositoryConfig;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::Node;
use raisin_models::translations::LocaleCode;
use raisin_storage::{ReadSnapshot, Storage};
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
    /// Read each frontier level in one batched call (`sql.batched_fetch`).
    batched_fetch: bool,
    /// The statement's storage view, shared by every batched read.
    read_snapshot: Option<ReadSnapshot>,
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
            batched_fetch: true,
            read_snapshot: None,
        }
    }

    /// `false` reads targets one at a time (the `sql.batched_fetch = false`
    /// rollback); the default reads each frontier level in one batched call.
    pub fn with_batched_fetch(mut self, batched: bool) -> Self {
        self.batched_fetch = batched;
        self
    }

    /// Read targets through the statement's storage snapshot, so every level
    /// (and every row of the statement) sees one view of the database. Must
    /// be a view taken at or after `snapshot`'s revision.
    pub fn with_read_snapshot(mut self, snapshot: Option<ReadSnapshot>) -> Self {
        self.read_snapshot = snapshot;
        self
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

    /// Resolve the references in several documents — the rows of one chunk of
    /// a statement — reading and producing stored VALUES, not JSON (plan
    /// Phase 13d). ONE frontier walk serves them all, so a level's targets
    /// across every document are read in one batch; each document is then
    /// inlined on its own.
    ///
    /// Each result is the value whose JSON rendering is the document the
    /// JSON resolver produced for the document's rendering (`doc.rs`); a
    /// caller that wants what that JSON reads back as applies
    /// `PropertyValue::into_json_round_trip`.
    pub async fn resolve_values_many(
        &self,
        workspace: &str,
        mut values: Vec<raisin_models::nodes::properties::PropertyValue>,
        max_depth: u32,
        fields: Option<&[String]>,
    ) -> Result<Vec<raisin_models::nodes::properties::PropertyValue>> {
        let depth = max_depth.min(MAX_RESOLUTION_DEPTH);
        if depth == 0 {
            return Ok(values);
        }
        values.iter_mut().for_each(doc::make_walkable);

        let read = Arc::new(ReadScope {
            snapshot: self.snapshot,
            locale: self
                .effective_locale()
                .map(|l| l.locale.as_str().to_string()),
            fields: fields.map(<[String]>::to_vec),
        });
        let resolved = self
            .gather(workspace, &values, depth, &read, fields)
            .await?;

        if resolved.values().any(Option::is_some) {
            for value in &mut values {
                let totals = walk::Inliner::new(
                    workspace,
                    &resolved,
                    self.memo.allowance(),
                    self.memo.budget(),
                )
                .inline(value, depth)?;
                self.memo.charge(totals)?;
            }
        }
        Ok(values)
    }

    /// The locale to translate targets into, or `None` when that is the base
    /// language anyway.
    fn effective_locale(&self) -> Option<&ResolutionLocale> {
        self.locale
            .as_ref()
            .filter(|l| l.locale.as_str() != l.default_language)
    }
}
