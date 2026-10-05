//! Translation COMMANDS: the one caller-facing implementation of translate,
//! delete-translation, hide-in-locale, unhide-in-locale and list.
//!
//! [`TranslationService`] is the storage-level primitive: it trusts its
//! arguments, takes a bare node id and an actor string, and checks nothing.
//! Every transport used to wrap it by hand, and the copies drifted — the
//! WebSocket one passed the node PATH where the store keys by node ID (so its
//! overlays were invisible to every reader), recorded the actor as `"system"`,
//! and checked no permission at all, while the HTTP one let the request body
//! name the actor. These methods are what a transport calls instead, so the
//! rules below hold whichever wire a write arrived on:
//!
//! - **The node is resolved through the RLS-filtered read** ([`NodeService::get_by_path`]
//!   / [`NodeService::get`]); a node the caller cannot read is "not found", never
//!   a distinguishable refusal.
//! - **A write requires `Update` on the node**, the same check
//!   [`NodeService::update_node`] makes: a translation is a change to the node's
//!   content in one locale, and hiding a node in a locale removes it from that
//!   locale entirely.
//! - **The overlay is keyed by the resolved node ID**, never by what the caller
//!   sent.
//! - **The actor is the authenticated caller** (`AuthContext::actor_id`), never a
//!   value from the request.
//!
//! Transports parse their wire format, call one of these, and map the result.

use std::collections::HashMap;

use raisin_error::{Error, Result};
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_models::permissions::Operation;
use raisin_models::translations::{JsonPointer, LocaleCode};
use raisin_storage::{
    transactional::TransactionalStorage, BranchRepository, NodeRepository, Storage,
};

use super::{TranslationService, TranslationUpdateResult};
use crate::services::node_service::NodeService;

/// How a translation command names its node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeRef<'a> {
    /// An absolute node path (`/a/b`).
    Path(&'a str),
    /// A node id.
    Id(&'a str),
}

impl<'a> NodeRef<'a> {
    /// Read a caller-supplied reference: an absolute path starts with `/`,
    /// anything else is a node id.
    ///
    /// The WebSocket payload field is called `node_path`, but because the old
    /// handler passed it straight through as the storage key, the only clients
    /// whose translations were ever visible were the ones sending an ID in it.
    /// Accepting both keeps those working and fixes the ones sending a path.
    pub fn parse(reference: &'a str) -> Self {
        if reference.starts_with('/') {
            NodeRef::Path(reference)
        } else {
            NodeRef::Id(reference)
        }
    }

    fn describe(&self) -> &'a str {
        match self {
            NodeRef::Path(p) | NodeRef::Id(p) => p,
        }
    }
}

/// The locales a node has translations for.
#[derive(Debug, Clone)]
pub struct NodeTranslations {
    /// The resolved node id.
    pub node_id: String,
    /// Locales with an overlay at the branch head.
    pub locales: Vec<LocaleCode>,
}

/// Turn a wire map of translated fields into the overlay input.
///
/// A key starting with `/` is a JSON pointer (`/title`, `/seo/description`); any
/// other key is a top-level property name and becomes `/<name>`. Values go
/// through [`PropertyValue::from_json`], the one JSON → property converter, so
/// the same JSON produces the same overlay whichever transport carried it.
pub fn parse_translation_fields(
    fields: impl IntoIterator<Item = (String, serde_json::Value)>,
) -> Result<HashMap<JsonPointer, PropertyValue>> {
    let mut out = HashMap::new();
    for (key, value) in fields {
        let pointer_str = if key.starts_with('/') {
            key
        } else {
            format!("/{key}")
        };
        let pointer = JsonPointer::parse(&pointer_str)
            .map_err(|e| Error::Validation(format!("Invalid JSON pointer {pointer_str}: {e}")))?;
        out.insert(pointer, PropertyValue::from_json(&value));
    }
    Ok(out)
}

impl<S: Storage + TransactionalStorage> NodeService<S> {
    /// Resolve `node` through the RLS-filtered read; unreadable is not found.
    async fn resolve_translation_target(&self, node: NodeRef<'_>) -> Result<Node> {
        let found = match node {
            NodeRef::Path(path) => self.get_by_path(path).await?,
            NodeRef::Id(id) => self.get(id).await?,
        };
        found.ok_or_else(|| Error::NotFound(format!("Node not found: {}", node.describe())))
    }

    /// Resolve `node` and require `Update` on it, exactly as `update_node` does.
    ///
    /// The check runs against the STORED node, not the read result: the read is
    /// field-filtered by RLS, and a permission condition over a field the caller
    /// cannot see must still evaluate against the real value. A draft that only
    /// exists as a workspace delta has no stored row, so its read result is used.
    async fn resolve_translation_write_target(&self, node: NodeRef<'_>) -> Result<Node> {
        let readable = self.resolve_translation_target(node).await?;
        let stored = self
            .storage
            .nodes()
            .get(self.scope(), &readable.id, self.revision.as_ref())
            .await?
            .unwrap_or(readable);
        if !self.check_rls_permission(&stored, Operation::Update).await {
            return Err(Error::PermissionDenied(format!(
                "Permission denied: cannot translate node '{}' at path '{}'",
                stored.id, stored.path
            )));
        }
        Ok(stored)
    }

    fn translation_service(&self) -> TranslationService<S> {
        TranslationService::new(self.storage.clone())
    }

    /// Merge `translations` into the node's overlay for `locale`.
    ///
    /// A `null` value clears that pointer (see
    /// [`TranslationService::update_translation`]).
    pub async fn translate(
        &self,
        node: NodeRef<'_>,
        locale: &LocaleCode,
        translations: HashMap<JsonPointer, PropertyValue>,
        message: Option<String>,
    ) -> Result<TranslationUpdateResult> {
        let target = self.resolve_translation_write_target(node).await?;
        self.translation_service()
            .update_translation(
                &self.tenant_id,
                &self.repo_id,
                &self.branch,
                &self.workspace_id,
                &target.id,
                locale,
                translations,
                &self.commit_actor(),
                message,
            )
            .await
    }

    /// Drop every translation the node has in `locale`.
    pub async fn delete_locale_translation(
        &self,
        node: NodeRef<'_>,
        locale: &LocaleCode,
        message: Option<String>,
    ) -> Result<TranslationUpdateResult> {
        let target = self.resolve_translation_write_target(node).await?;
        self.translation_service()
            .delete_translation(
                &self.tenant_id,
                &self.repo_id,
                &self.branch,
                &self.workspace_id,
                &target.id,
                locale,
                &self.commit_actor(),
                message,
            )
            .await
    }

    /// Hide the node in `locale`: reads in that locale omit it.
    pub async fn hide_in_locale(
        &self,
        node: NodeRef<'_>,
        locale: &LocaleCode,
        message: Option<String>,
    ) -> Result<TranslationUpdateResult> {
        let target = self.resolve_translation_write_target(node).await?;
        self.translation_service()
            .hide_node(
                &self.tenant_id,
                &self.repo_id,
                &self.branch,
                &self.workspace_id,
                &target.id,
                locale,
                &self.commit_actor(),
                message,
            )
            .await
    }

    /// Make the node visible again in `locale`.
    ///
    /// Like delete, this stores an empty overlay — a hidden node has no
    /// translated fields to keep, because hiding replaced them.
    pub async fn unhide_in_locale(
        &self,
        node: NodeRef<'_>,
        locale: &LocaleCode,
        message: Option<String>,
    ) -> Result<TranslationUpdateResult> {
        let target = self.resolve_translation_write_target(node).await?;
        self.translation_service()
            .unhide_node(
                &self.tenant_id,
                &self.repo_id,
                &self.branch,
                &self.workspace_id,
                &target.id,
                locale,
                &self.commit_actor(),
                Some(message.unwrap_or_else(|| "Unhide node".to_string())),
            )
            .await
    }

    /// The locales the node has an overlay for at the branch head.
    ///
    /// Read access to the node is enough.
    pub async fn list_node_translations(&self, node: NodeRef<'_>) -> Result<NodeTranslations> {
        let target = self.resolve_translation_target(node).await?;
        let revision = match self.revision {
            Some(rev) => rev,
            None => {
                self.storage
                    .branches()
                    .get_head(&self.tenant_id, &self.repo_id, &self.branch)
                    .await?
            }
        };
        let locales = self
            .translation_service()
            .list_translations(
                &self.tenant_id,
                &self.repo_id,
                &self.branch,
                &self.workspace_id,
                &target.id,
                &revision,
            )
            .await?;
        Ok(NodeTranslations {
            node_id: target.id,
            locales,
        })
    }
}
