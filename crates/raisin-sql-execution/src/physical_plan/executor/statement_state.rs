//! State that lives exactly as long as one statement.
//!
//! An `ExecutionContext` is built per statement and cloned into every operator
//! of its plan; this is the part of it the clones SHARE. It holds things that
//! are wrong to keep longer — RESOLVE's memo carries rows already filtered for
//! one caller at one snapshot — and things that are wasteful to rebuild per row.

use super::context::ExecutionContext;
use raisin_core::services::reference_resolver::ResolveMemo;
use raisin_core::services::translation_resolver::TranslationResolver;
use raisin_error::Error;
use raisin_hlc::HLC;
use raisin_storage::{BranchRepository, Storage};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

/// See the module docs. Shared by every clone of one statement's context.
pub struct StatementState<S: Storage> {
    /// RESOLVE's targets and budget, shared by every row of the statement.
    resolve_memo: Arc<ResolveMemo>,
    /// The revision every read of the statement is pinned to, when the
    /// context was not given one.
    snapshot: tokio::sync::OnceCell<HLC>,
    /// Built on first use, then shared by every row that translates.
    translation_resolver: OnceLock<Arc<TranslationResolver<S::Translations>>>,
    /// `embedding` was named but no embedding store exists: said once.
    embedding_unavailable_warned: AtomicBool,
}

impl<S: Storage> Default for StatementState<S> {
    fn default() -> Self {
        Self {
            resolve_memo: Arc::new(ResolveMemo::default()),
            snapshot: tokio::sync::OnceCell::new(),
            translation_resolver: OnceLock::new(),
            embedding_unavailable_warned: AtomicBool::new(false),
        }
    }
}

impl<S: Storage> ExecutionContext<S> {
    /// The revision this statement reads at: `max_revision` when the query
    /// pinned one (the engine pins every query to HEAD), otherwise the branch
    /// HEAD, read ONCE and reused for the rest of the statement — so two reads
    /// of one statement cannot straddle a concurrent commit.
    pub async fn statement_snapshot(&self) -> Result<HLC, Error> {
        if let Some(revision) = self.max_revision {
            return Ok(revision);
        }
        self.statement
            .snapshot
            .get_or_try_init(|| async {
                let branch = self
                    .storage
                    .branches()
                    .get_branch(&self.tenant_id, &self.repo_id, &self.branch)
                    .await?;
                Ok::<_, Error>(branch.map(|b| b.head).unwrap_or_else(|| HLC::new(0, 0)))
            })
            .await
            .copied()
    }

    /// RESOLVE's memo for this statement.
    pub fn resolve_memo(&self) -> Arc<ResolveMemo> {
        self.statement.resolve_memo.clone()
    }

    /// The statement's translation resolver, or `None` when the repository has
    /// no translation configuration. One per statement, not one per row.
    pub fn translation_resolver(&self) -> Option<Arc<TranslationResolver<S::Translations>>> {
        let config = self.repository_config.as_ref()?;
        Some(
            self.statement
                .translation_resolver
                .get_or_init(|| {
                    Arc::new(TranslationResolver::new(
                        Arc::new(self.storage.translations().clone()),
                        config.clone(),
                    ))
                })
                .clone(),
        )
    }

    /// True the first time it is asked in this statement, false afterwards.
    pub(crate) fn first_embedding_unavailable_warning(&self) -> bool {
        !self
            .statement
            .embedding_unavailable_warned
            .swap(true, Ordering::Relaxed)
    }
}
