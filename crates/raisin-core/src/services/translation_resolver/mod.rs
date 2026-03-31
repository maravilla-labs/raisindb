//! Translation resolution service for applying locale-specific translations to nodes.
//!
//! This module provides the core translation resolution logic that:
//! - Applies configurable locale fallback chains (e.g., fr-CA -> fr -> en)
//! - Merges LocaleOverlay data with base nodes
//! - Handles Hidden tombstone markers (hiding nodes in specific locales)
//! - Resolves block-level translations by UUID for Composite properties

mod block_resolution;

use raisin_context::RepositoryConfig;
use raisin_error::{Error, Result};
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_models::translations::{JsonPointer, LocaleCode, LocaleOverlay};
use raisin_storage::TranslationRepository;
use std::collections::HashMap;
use std::sync::Arc;

/// Translation resolver service that applies locale-specific translations to nodes.
pub struct TranslationResolver<R: TranslationRepository> {
    repository: Arc<R>,
    config: RepositoryConfig,
}

impl<R: TranslationRepository> TranslationResolver<R> {
    /// Create a new translation resolver with the given repository and config.
    pub fn new(repository: Arc<R>, config: RepositoryConfig) -> Self {
        Self { repository, config }
    }

    /// Resolve a node with translations for the given locale.
    ///
    /// Applies the locale fallback chain to merge translations into the base node.
    /// If the node is hidden in any locale in the chain, returns None.
    pub async fn resolve_node(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        mut node: Node,
        locale: &LocaleCode,
        revision: &raisin_hlc::HLC,
    ) -> Result<Option<Node>> {
        let fallback_chain = self.config.get_fallback_chain(locale.as_str());

        for fallback_locale in fallback_chain {
            let locale_code = LocaleCode::parse(&fallback_locale)?;

            let overlay = self
                .repository
                .get_translation(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    &node.id,
                    &locale_code,
                    revision,
                )
                .await?;

            if let Some(overlay) = overlay {
                match overlay {
                    LocaleOverlay::Hidden => {
                        return Ok(None);
                    }
                    LocaleOverlay::Properties { data } => {
                        self.apply_overlay(
                            &mut node,
                            data,
                            tenant_id,
                            repo_id,
                            branch,
                            workspace,
                            &locale_code,
                            revision,
                        )
                        .await?;
                    }
                }
            }
        }

        Ok(Some(node))
    }

    /// Apply a translation overlay to a node's properties.
    async fn apply_overlay(
        &self,
        node: &mut Node,
        overlay: HashMap<JsonPointer, PropertyValue>,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        locale: &LocaleCode,
        revision: &raisin_hlc::HLC,
    ) -> Result<()> {
        for (pointer, value) in overlay {
            self.merge_property(node, &pointer, value)?;
        }

        self.resolve_block_translations(
            node, tenant_id, repo_id, branch, workspace, locale, revision,
        )
        .await?;

        Ok(())
    }

    /// Merge a single property value into the node using a JsonPointer path.
    fn merge_property(
        &self,
        node: &mut Node,
        pointer: &JsonPointer,
        value: PropertyValue,
    ) -> Result<()> {
        let segments = pointer.segments();
        if segments.is_empty() {
            return Err(Error::Validation(
                "Cannot merge empty JsonPointer path".to_string(),
            ));
        }
        merge_into_map(&mut node.properties, &segments, value)
    }

    /// Batch resolve multiple nodes with translations for the given locale.
    ///
    /// Uses batch translation fetching for 10-100x better performance
    /// than calling `resolve_node` individually for each node.
    pub async fn resolve_nodes_batch(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        nodes: Vec<Node>,
        locale: &LocaleCode,
        revision: &raisin_hlc::HLC,
    ) -> Result<Vec<Node>> {
        if nodes.is_empty() {
            return Ok(Vec::new());
        }

        let fallback_chain = self.config.get_fallback_chain(locale.as_str());
        let node_ids: Vec<String> = nodes.iter().map(|n| n.id.clone()).collect();

        let mut nodes_by_id: HashMap<String, Node> =
            nodes.into_iter().map(|n| (n.id.clone(), n)).collect();
        let mut hidden_nodes: std::collections::HashSet<String> = std::collections::HashSet::new();

        for fallback_locale in fallback_chain {
            let locale_code = LocaleCode::parse(&fallback_locale)?;

            let translations = self
                .repository
                .get_translations_batch(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    &node_ids,
                    &locale_code,
                    revision,
                )
                .await?;

            for (node_id, overlay) in translations {
                if hidden_nodes.contains(&node_id) {
                    continue;
                }

                match overlay {
                    LocaleOverlay::Hidden => {
                        hidden_nodes.insert(node_id.clone());
                        nodes_by_id.remove(&node_id);
                    }
                    LocaleOverlay::Properties { data } => {
                        if let Some(node) = nodes_by_id.get_mut(&node_id) {
                            self.apply_overlay(
                                node,
                                data,
                                tenant_id,
                                repo_id,
                                branch,
                                workspace,
                                &locale_code,
                                revision,
                            )
                            .await?;
                        }
                    }
                }
            }
        }

        let result: Vec<Node> = node_ids
            .into_iter()
            .filter_map(|id| nodes_by_id.remove(&id))
            .collect();

        Ok(result)
    }
}

/// Recursively merge a value into a property map following the given path segments.
///
/// Handles both `Object` (navigate by key) and `Array` (navigate by UUID) intermediate values.
/// For arrays, the next segment is matched against the `uuid` field of each object element.
pub(super) fn merge_into_map(
    current: &mut HashMap<String, PropertyValue>,
    segments: &[&str],
    value: PropertyValue,
) -> Result<()> {
    if segments.len() == 1 {
        current.insert(segments[0].to_string(), value);
        return Ok(());
    }

    let segment = segments[0];
    let remaining = &segments[1..];

    if !current.contains_key(segment) {
        current.insert(segment.to_string(), PropertyValue::Object(HashMap::new()));
    }

    match current.get_mut(segment) {
        Some(PropertyValue::Object(obj)) => merge_into_map(obj, remaining, value),
        Some(PropertyValue::Array(arr)) => {
            // Array navigation: next segment is a UUID, rest are field path
            let uuid = remaining[0];
            let field_segments = &remaining[1..];
            for item in arr.iter_mut() {
                match item {
                    PropertyValue::Object(obj) => {
                        if obj.get("uuid") == Some(&PropertyValue::String(uuid.to_string())) {
                            return if field_segments.is_empty() {
                                Ok(()) // replacing whole block — no-op
                            } else {
                                merge_into_map(obj, field_segments, value)
                            };
                        }
                    }
                    PropertyValue::Element(element) => {
                        if element.uuid == uuid {
                            return if field_segments.is_empty() {
                                Ok(()) // replacing whole block — no-op
                            } else {
                                merge_into_map(&mut element.content, field_segments, value)
                            };
                        }
                    }
                    _ => {}
                }
            }
            Ok(()) // UUID not found — skip silently
        }
        Some(_) => Err(Error::Validation(format!(
            "Cannot navigate through non-object property at path segment '{}'",
            segment
        ))),
        None => unreachable!("We just inserted this key"),
    }
}

#[cfg(test)]
mod tests;
