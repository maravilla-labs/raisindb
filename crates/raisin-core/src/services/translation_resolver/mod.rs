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
        // ONE read gives every block overlay of this node in the whole chain, AS
        // OF `revision`. Almost no node has any, and the answer is reused for
        // every locale in the chain — where the old shape walked the node's whole
        // property tree and issued a point read per block uuid it found, per
        // locale. It used to be a HEAD listing followed by a point read per
        // block: a time-travel read lost every block a later delete of the node
        // ended, and each point read repeated the node's delete check.
        let chain = parse_chain(&fallback_chain)?;
        let block_overlays = self
            .repository
            .get_block_translations_for_node(
                tenant_id, repo_id, branch, workspace, &node.id, &chain, revision,
            )
            .await?;

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
            self.apply_block_overlays_for_locale(&mut node, &block_overlays, &locale_code)?;
        }

        Ok(Some(node))
    }

    /// Is the node visible in `locale` — i.e. not hidden anywhere in its
    /// fallback chain — without merging any overlay?
    ///
    /// For readers that never look at the translated properties (a listing
    /// that projects `path`): [`Self::resolve_node`] answers the same question
    /// but also reads every block overlay and merges into the property tree.
    #[allow(clippy::too_many_arguments)]
    pub async fn is_visible(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        locale: &LocaleCode,
        revision: &raisin_hlc::HLC,
    ) -> Result<bool> {
        for fallback_locale in self.config.get_fallback_chain(locale.as_str()) {
            let locale_code = LocaleCode::parse(&fallback_locale)?;
            let overlay = self
                .repository
                .get_translation(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    node_id,
                    &locale_code,
                    revision,
                )
                .await?;
            if matches!(overlay, Some(LocaleOverlay::Hidden)) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Apply the block overlays that belong to ONE locale of the fallback chain.
    ///
    /// `block_overlays` is every block overlay of the node in the chain, read
    /// once by the caller at the read revision. The node's property tree is not
    /// walked unless this locale actually has a block overlay, which is what
    /// makes the common case (no block overlays anywhere) free.
    fn apply_block_overlays_for_locale(
        &self,
        node: &mut Node,
        block_overlays: &[(String, LocaleCode, LocaleOverlay)],
        locale: &LocaleCode,
    ) -> Result<()> {
        for (block_uuid, overlay_locale, overlay) in block_overlays {
            if overlay_locale != locale {
                continue;
            }
            if let LocaleOverlay::Properties { data } = overlay {
                self.apply_block_translation_by_uuid(
                    &mut node.properties,
                    block_uuid,
                    data.clone(),
                )?;
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
        let chain: Vec<LocaleCode> = parse_chain(&self.config.get_fallback_chain(locale.as_str()))?
            .into_iter()
            .rev()
            .collect();

        for node in nodes_by_id.values_mut() {
            let block_overlays = self
                .repository
                .get_block_translations_for_node(
                    tenant_id, repo_id, branch, workspace, &node.id, &chain, revision,
                )
                .await?;
            if block_overlays.is_empty() {
                continue;
            }
            for locale_code in &chain {
                self.apply_block_overlays_for_locale(node, &block_overlays, locale_code)?;
            }
        }

        let result: Vec<Node> = node_ids
            .into_iter()
            .filter_map(|id| nodes_by_id.remove(&id))
            .collect();

        Ok(result)
    }
}

/// The fallback chain as locale codes, in the chain's order.
fn parse_chain(chain: &[String]) -> Result<Vec<LocaleCode>> {
    chain.iter().map(|l| LocaleCode::parse(l)).collect()
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
        // An ELEMENT held directly on a property — an `ElementField`, e.g. a page's
        // `hero` — is a keyed container like an object, and its fields are addressed
        // by name: `/hero/headline`. Only elements INSIDE an array were handled, so
        // the moment anyone translated an embedded element the whole localized read
        // failed with "Cannot navigate through non-object property" — not the field
        // skipped, the entire page 400ing in that language.
        Some(PropertyValue::Element(element)) => {
            merge_into_map(&mut element.content, remaining, value)
        }
        // A COMPOSITE navigates by item uuid, exactly like an array.
        Some(PropertyValue::Composite(composite)) => {
            let uuid = remaining[0];
            let field_segments = &remaining[1..];
            for item in composite.items.iter_mut() {
                if item.uuid == uuid {
                    return if field_segments.is_empty() {
                        Ok(())
                    } else {
                        merge_into_map(&mut item.content, field_segments, value)
                    };
                }
            }
            Ok(()) // uuid not found — skip, as the array case does
        }
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
        // A pointer that cannot be navigated is a STALE pointer, not a corrupt node:
        // the content changed shape under a translation written against the old one.
        // Skip that field — it falls back to the base language — rather than failing
        // the read, which took the whole page down in that locale and left no way to
        // fix it from the editor. Consistent with the uuid-not-found cases above and
        // with the renderer-side overlay, which also skips unknown pointers.
        Some(_) => {
            tracing::debug!(
                "translation overlay skipped: cannot navigate path segment '{}'",
                segment
            );
            Ok(())
        }
        None => unreachable!("We just inserted this key"),
    }
}

#[cfg(test)]
mod tests;
