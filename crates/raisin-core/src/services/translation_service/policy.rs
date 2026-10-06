//! WHO may translate WHICH node: the one decision every translation write
//! goes through, whichever surface it arrived on.
//!
//! The HTTP and WebSocket commands ([`super::commands`]) and SQL's
//! `UPDATE … FOR LOCALE` all call [`NodeService::authorize_translation_write`].
//! They differ only in how they WRITE (SQL stages the overlay in its
//! transaction, so it commits atomically with the statements around it; the
//! commands go through [`super::TranslationService`]) — never in what they let
//! through. SQL used to carry its own copy of this check, and it had drifted:
//! it looked the node up without RLS, so a node the caller could not read
//! answered "permission denied" (confirming it exists) instead of "not found",
//! a caller holding `Translate` without `Read` could write to it, drafts in the
//! workspace delta were invisible to it, and it attributed the write by its
//! own actor rule.
//!
//! The rules:
//!
//! - **The node is resolved through the RLS-filtered read** ([`NodeService::get_by_path`]
//!   / [`NodeService::get`]); a node the caller cannot read is "not found", never
//!   a distinguishable refusal.
//! - **A write requires the `Translate` permission on the STORED node**, graph
//!   (`RELATES … VIA`) conditions included — [`NodeService::check_rls_permission`]
//!   builds the resolver when the caller's grants need one. Translating is its
//!   own grant: `Update` alone does not imply it.
//! - **The overlay is keyed by the resolved node ID**, never by what the caller
//!   sent.
//! - **The actor is the authenticated caller** (`AuthContext::actor_id`), never a
//!   value from the request.
//! - **No auth context denies.** A surface that runs as the system when it has
//!   no caller (SQL) must say so by passing [`AuthContext::system`] explicitly.
//!
//! [`AuthContext::system`]: raisin_models::auth::AuthContext::system

use raisin_error::{Error, Result};
use raisin_models::nodes::Node;
use raisin_models::permissions::Operation;
use raisin_storage::{transactional::TransactionalStorage, NodeRepository, Storage};

use super::commands::NodeRef;
use crate::services::node_service::NodeService;

/// A node the caller may translate, and who the write is attributed to.
#[derive(Debug, Clone)]
pub struct TranslationWriteTarget {
    /// The stored node; key the overlay by `node.id`.
    pub node: Node,
    /// The actor to record on the translation's history.
    pub actor: String,
}

impl<S: Storage + TransactionalStorage> NodeService<S> {
    /// Resolve `node` through the RLS-filtered read; unreadable is not found.
    pub(super) async fn resolve_translation_target(&self, node: NodeRef<'_>) -> Result<Node> {
        let found = match node {
            NodeRef::Path(path) => self.get_by_path(path).await?,
            NodeRef::Id(id) => self.get(id).await?,
        };
        found.ok_or_else(|| Error::NotFound(format!("Node not found: {}", node.describe())))
    }

    /// May this service's caller translate `node`? Resolve it, require
    /// `Translate` on it, and name the actor.
    ///
    /// Errors are [`Error::NotFound`] (missing or unreadable) and
    /// [`Error::PermissionDenied`] (readable, not translatable); every surface
    /// maps those two the same way.
    ///
    /// The check runs against the STORED node, not the read result: the read is
    /// field-filtered by RLS, and a permission condition over a field the caller
    /// cannot see must still evaluate against the real value. A draft that only
    /// exists as a workspace delta has no stored row, so its read result is used.
    pub async fn authorize_translation_write(
        &self,
        node: NodeRef<'_>,
    ) -> Result<TranslationWriteTarget> {
        let readable = self.resolve_translation_target(node).await?;
        let stored = self
            .storage
            .nodes()
            .get(self.scope(), &readable.id, self.revision.as_ref())
            .await?
            .unwrap_or(readable);
        if !self
            .check_rls_permission(&stored, Operation::Translate)
            .await
        {
            return Err(Error::PermissionDenied(format!(
                "Permission denied: cannot translate node '{}' at path '{}'",
                stored.id, stored.path
            )));
        }
        Ok(TranslationWriteTarget {
            node: stored,
            actor: self.commit_actor(),
        })
    }
}
