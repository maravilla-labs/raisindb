//! What an uncommitted transaction has written, as sibling uniqueness sees
//! it (plan Phase 13c).
//!
//! A transaction stages its nodes and overlays in one batch that nothing can
//! read before it is written. Checked against stored state alone, two
//! siblings written by ONE transaction (a package install creating `/a` and
//! naming `/b` `a` in French; `UPDATE … FOR LOCALE … SET __node_name = 'x'`
//! over several siblings) both passed, and a stored sibling the transaction
//! renamed, moved or deleted still counted with its stored name. So every
//! sibling the check finds — stored or staged — is judged by its FINAL view:
//! the transaction's version where it wrote one, the stored one otherwise.
//!
//! The localized name module knows nothing about transactions: the
//! transaction implements [`PendingWrites`] over its read cache, and every
//! other caller passes [`NoPending`].

use crate::localized_name::keys::NameScope;
use crate::localized_name::lookup::view::NodeView;
use crate::localized_name::reads::overlay_over;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use raisin_models::translations::LocaleOverlay;
use rocksdb::DB;
use std::collections::BTreeMap;

/// The uncommitted writes of one transaction in one workspace.
pub(crate) trait PendingWrites {
    /// The node as the transaction leaves it: `Some(Some)` written or moved,
    /// `Some(None)` deleted, `None` not touched (its stored version stands).
    fn node(&self, id: &str) -> Option<Option<Node>>;
    /// Its overlay in `locale` as the transaction leaves it: `Some(None)`
    /// deleted, `None` not written by the transaction.
    fn overlay(&self, id: &str, locale: &str) -> Option<Option<LocaleOverlay>>;
    /// Who sits at `path` after the transaction: `Some(None)` vacated by it,
    /// `None` not touched (the path index answers).
    fn at_path(&self, path: &str) -> Option<Option<String>>;
    /// Nodes whose overlays in this transaction set `name` in `locale`.
    /// HINTS: a later overlay may have replaced the name, so every one is
    /// judged by its final view.
    fn named(&self, locale: &str, name: &str) -> Vec<String>;
    /// The revision its overlays are staged at: a stored version NEWER than
    /// it wins on read, so it wins in the final view too (`overlay_over`).
    fn staged_at(&self) -> HLC;
}

/// No uncommitted writes: every write path outside a transaction.
pub(crate) struct NoPending;

impl PendingWrites for NoPending {
    fn node(&self, _: &str) -> Option<Option<Node>> {
        None
    }
    fn overlay(&self, _: &str, _: &str) -> Option<Option<LocaleOverlay>> {
        None
    }
    fn at_path(&self, _: &str) -> Option<Option<String>> {
        None
    }
    fn named(&self, _: &str, _: &str) -> Vec<String> {
        Vec::new()
    }
    fn staged_at(&self) -> HLC {
        crate::mvcc_read::NEWEST
    }
}

/// `id` with its overlays in `chain` as the transaction leaves them (stored
/// state where it wrote nothing); `None` when it does not exist then.
pub(super) fn final_view(
    db: &DB,
    scope: NameScope<'_>,
    pending: &dyn PendingWrites,
    id: &str,
    chain: &[String],
) -> Result<Option<NodeView>> {
    let staged: Vec<(String, Option<LocaleOverlay>)> = chain
        .iter()
        .filter_map(|locale| pending.overlay(id, locale).map(|o| (locale.clone(), o)))
        .collect();
    let (node, revision) = match pending.node(id) {
        Some(None) => return Ok(None),
        Some(Some(node)) => (node, crate::mvcc_read::NEWEST),
        None => {
            let Some(view) = NodeView::load(db, scope, id, chain, None)? else {
                return Ok(None);
            };
            if staged.is_empty() {
                return Ok(Some(view));
            }
            (view.node, view.revision)
        }
    };
    let mut overlays: BTreeMap<String, Option<LocaleOverlay>> = BTreeMap::new();
    for locale in chain {
        let overlay = match staged.iter().find(|(l, _)| l == locale) {
            // The rule the index writer and the checked node's own view
            // apply (`overlays_at`): a newer stored version beats the staged.
            Some((_, overlay)) => {
                overlay_over(db, scope, id, locale, None, overlay, &pending.staged_at())?
            }
            None => crate::translation_read::read_overlay(
                db,
                scope.tenant_id,
                scope.repo_id,
                scope.branch,
                scope.workspace,
                id,
                locale,
                None,
            )?,
        };
        overlays.insert(locale.clone(), overlay);
    }
    Ok(Some(NodeView::from_parts(node, revision, overlays)))
}
