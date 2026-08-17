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

        // LEAST specific first, so the requested locale is applied LAST and wins.
        //
        // The chain is ordered most-specific-first (`fr-CA`, `fr`, `en`), and each
        // overlay is merged over the node as it is found — so walking it forwards
        // let `fr` overwrite the `fr-CA` values that had just been applied, i.e.
        // exactly backwards. A field the more specific locale did not translate
        // still shows through, because the less specific overlay was applied
        // underneath it rather than instead of it.
        //
        // `Hidden` is order-independent: hidden anywhere in the chain hides the node.
        //
        // ONE scan tells us whether this node has block overlays at all. Almost no
        // node does, and the answer is reused for every locale in the chain — where
        // the old shape walked the node's whole property tree and issued a point
        // read per block uuid it found, per locale, only to learn "none" every time.
        let block_overlays = self
            .repository
            .list_block_translations_for_node(tenant_id, repo_id, branch, workspace, &node.id)
            .await
            .unwrap_or_default();

        for fallback_locale in fallback_chain.into_iter().rev() {
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
                        for (pointer, value) in data {
                            self.merge_property(&mut node, &pointer, value)?;
                        }
                    }
                }
            }

            // Block overlays are applied for this locale WHETHER OR NOT the node
            // itself has one. They used to hang off the node-overlay branch above,
            // so a block translated in a locale where the node had no overlay of
            // its own was stored, listed, and never resolved.
            self.apply_block_overlays_for_locale(
                &mut node,
                &block_overlays,
                tenant_id,
                repo_id,
                branch,
                workspace,
                &locale_code,
                revision,
            )
            .await?;
        }

        Ok(Some(node))
    }

    /// Apply the block overlays that belong to ONE locale of the fallback chain.
    ///
    /// `block_overlays` is the node's `(block_uuid, locale)` inventory, read once by
    /// the caller. Nothing is fetched — and the node's property tree is not walked —
    /// unless this locale actually has a block overlay, which is what makes the
    /// common case (no block overlays anywhere) free rather than N reads per locale.
    #[allow(clippy::too_many_arguments)]
    async fn apply_block_overlays_for_locale(
        &self,
        node: &mut Node,
        block_overlays: &[(String, LocaleCode)],
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        locale: &LocaleCode,
        revision: &raisin_hlc::HLC,
    ) -> Result<()> {
        for (block_uuid, overlay_locale) in block_overlays {
            if overlay_locale != locale {
                continue;
            }
            let block_overlay = self
                .repository
                .get_block_translation(
                    tenant_id, repo_id, branch, workspace, &node.id, block_uuid, locale, revision,
                )
                .await?;
            if let Some(LocaleOverlay::Properties { data }) = block_overlay {
                self.apply_block_translation_by_uuid(&mut node.properties, block_uuid, data)?;
            }
        }

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

        // Same ordering rule as `resolve_node`: least specific first, so the
        // requested locale is applied last and wins over what it falls back to.
        for fallback_locale in fallback_chain.into_iter().rev() {
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
                            for (pointer, value) in data {
                                let segments = pointer.segments();
                                if segments.is_empty() {
                                    continue;
                                }
                                merge_into_map(&mut node.properties, &segments, value)?;
                            }
                        }
                    }
                }
            }
        }

        // Block overlays, once per surviving node. Kept OUT of the locale loop
        // above: the inventory scan is per node, not per locale, and a node's
        // block overlays are applied in the same least-specific-first order.
        let chain: Vec<LocaleCode> = self
            .config
            .get_fallback_chain(locale.as_str())
            .into_iter()
            .rev()
            .map(|l| LocaleCode::parse(&l))
            .collect::<Result<Vec<_>>>()?;

        for node in nodes_by_id.values_mut() {
            let block_overlays = self
                .repository
                .list_block_translations_for_node(tenant_id, repo_id, branch, workspace, &node.id)
                .await
                .unwrap_or_default();
            if block_overlays.is_empty() {
                continue;
            }
            for locale_code in &chain {
                self.apply_block_overlays_for_locale(
                    node,
                    &block_overlays,
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    locale_code,
                    revision,
                )
                .await?;
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
