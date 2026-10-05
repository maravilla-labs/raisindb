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
use raisin_storage::{ChainOverlays, TranslationRepository};
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
        node: Node,
        locale: &LocaleCode,
        revision: &raisin_hlc::HLC,
    ) -> Result<Option<Node>> {
        let mut resolved = self
            .resolve_nodes(
                tenant_id,
                repo_id,
                branch,
                workspace,
                vec![node],
                locale,
                revision,
            )
            .await?;
        Ok(resolved.pop().flatten())
    }

    /// [`Self::resolve_node`] for every node of a page, in ONE storage read
    /// (`get_chain_overlays`): the answer for each node, aligned with
    /// `nodes` — `None` where the node is hidden in the chain.
    ///
    /// This is THE resolution: `resolve_node` is a page of one, and
    /// `resolve_nodes_batch` filters this. A scan resolving row by row paid
    /// several iterator opens and a walk of the node's history per row and
    /// chain locale; a page pays one iterator per column family.
    #[allow(clippy::too_many_arguments)]
    pub async fn resolve_nodes(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        nodes: Vec<Node>,
        locale: &LocaleCode,
        revision: &raisin_hlc::HLC,
    ) -> Result<Vec<Option<Node>>> {
        Ok(self
            .resolve_page(
                tenant_id, repo_id, branch, workspace, nodes, locale, revision, false, true,
            )
            .await?
            .nodes)
    }

    /// [`Self::resolve_nodes`] (or, without `with_properties`, the
    /// visibility question alone), also handing back each node's node-level
    /// chain overlays when `keep_overlays` — what the localized name columns
    /// of the same rows are decided from, so they need not be read twice.
    #[allow(clippy::too_many_arguments)]
    pub async fn resolve_page(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        nodes: Vec<Node>,
        locale: &LocaleCode,
        revision: &raisin_hlc::HLC,
        keep_overlays: bool,
        with_properties: bool,
    ) -> Result<ResolvedPage> {
        let chain_names = self.config.get_fallback_chain(locale.as_str());
        if nodes.is_empty() {
            return Ok(ResolvedPage {
                chain: chain_names,
                ..ResolvedPage::default()
            });
        }
        // The chain is ordered most-specific-first (`fr-CA`, `fr`, `en`).
        let chain = parse_chain(&chain_names)?;
        // ONE read gives, per node, every node overlay and (when the
        // properties are merged) every block overlay of the whole chain AS OF
        // `revision`. Almost no node has block overlays, and the answer is
        // reused for every locale in the chain — where the old shape walked
        // the node's whole property tree and issued a point read per block
        // uuid it found, per locale. (It used to be a HEAD listing followed by
        // a point read per block: a time-travel read lost every block a later
        // delete of the node ended.)
        let ids: Vec<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
        let overlays = self
            .repository
            .get_chain_overlays(
                tenant_id,
                repo_id,
                branch,
                workspace,
                &ids,
                &chain,
                revision,
                with_properties,
            )
            .await?;
        let mut page = ResolvedPage {
            chain: chain_names,
            nodes: Vec::with_capacity(nodes.len()),
            overlays: Vec::new(),
        };
        for (node, overlays) in nodes.into_iter().zip(overlays) {
            if keep_overlays {
                page.overlays.push(overlays.node.clone());
            }
            page.nodes.push(if with_properties {
                self.apply_chain(node, &chain, overlays)?
            } else {
                (!hidden_in_chain(&overlays.node)).then_some(node)
            });
        }
        Ok(page)
    }

    /// Merge one node's chain overlays into it.
    ///
    /// LEAST specific first, so the requested locale is applied LAST and wins.
    /// Walking the chain forwards let `fr` overwrite the `fr-CA` values that
    /// had just been applied, i.e. exactly backwards. A field the more
    /// specific locale did not translate still shows through, because the
    /// less specific overlay was applied underneath it rather than instead of
    /// it. `Hidden` is order-independent: hidden anywhere in the chain hides
    /// the node.
    fn apply_chain(
        &self,
        mut node: Node,
        chain: &[LocaleCode],
        overlays: ChainOverlays,
    ) -> Result<Option<Node>> {
        let ChainOverlays {
            node: node_overlays,
            blocks: block_overlays,
        } = overlays;
        for (locale_code, overlay) in chain.iter().zip(node_overlays).rev() {
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
            self.apply_block_overlays_for_locale(&mut node, &block_overlays, locale_code)?;
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
        let visible = self
            .visible_nodes(
                tenant_id,
                repo_id,
                branch,
                workspace,
                &[node_id],
                locale,
                revision,
            )
            .await?;
        Ok(visible.first().copied().unwrap_or(true))
    }

    /// [`Self::is_visible`] for a page of nodes in one storage read, aligned
    /// with `node_ids`. No block overlay is read.
    #[allow(clippy::too_many_arguments)]
    pub async fn visible_nodes(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_ids: &[&str],
        locale: &LocaleCode,
        revision: &raisin_hlc::HLC,
    ) -> Result<Vec<bool>> {
        if node_ids.is_empty() {
            return Ok(Vec::new());
        }
        let chain = parse_chain(&self.config.get_fallback_chain(locale.as_str()))?;
        let overlays = self
            .repository
            .get_chain_overlays(
                tenant_id, repo_id, branch, workspace, node_ids, &chain, revision, false,
            )
            .await?;
        Ok(overlays.iter().map(|o| !hidden_in_chain(&o.node)).collect())
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

    /// Batch resolve multiple nodes with translations for the given locale,
    /// dropping the ones hidden in it: [`Self::resolve_nodes`], filtered.
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
        Ok(self
            .resolve_nodes(
                tenant_id, repo_id, branch, workspace, nodes, locale, revision,
            )
            .await?
            .into_iter()
            .flatten()
            .collect())
    }
}

/// A page of nodes as [`TranslationResolver::resolve_page`] resolved it.
#[derive(Debug, Default)]
pub struct ResolvedPage {
    /// The fallback chain, most specific first.
    pub chain: Vec<String>,
    /// Each node resolved, aligned with the input; `None`: hidden.
    pub nodes: Vec<Option<Node>>,
    /// Each node's node-level overlay per chain locale (empty unless kept).
    pub overlays: Vec<Vec<Option<LocaleOverlay>>>,
}

/// Hidden anywhere in the chain hides the node.
fn hidden_in_chain(overlays: &[Option<LocaleOverlay>]) -> bool {
    overlays
        .iter()
        .any(|overlay| matches!(overlay, Some(LocaleOverlay::Hidden)))
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
