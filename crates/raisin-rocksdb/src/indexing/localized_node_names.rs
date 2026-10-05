//! THE selector of the localized name index (plan Phase 12): a node's URL
//! segment in each locale — its TRANSLATED node name.
//!
//! Every writer, the rebuild, the fallback lookup and the lookup's candidate
//! verification call these two functions, so "what is this node's name in
//! `fr`" has exactly one answer everywhere. Selection is STRUCTURAL: the
//! node's own overlays and the repository's default language, never a
//! NodeType (the write path must not resolve schema) and never a property of
//! the base node (owner decision, 2026-10-04: no `url_{locale}` source).
//!
//! Per locale `L` (the default language never has one: its segment is the
//! canonical name):
//!
//! 1. `L`'s overlay is `Hidden` -> the node has NO segment in `L` (and is not
//!    visible there);
//! 2. `L`'s overlay sets the reserved pointer `/__node_name` to a string ->
//!    that;
//! 3. otherwise none: its segment in `L` is its canonical name.
//!
//! A value is a name when, trimmed of surrounding `/` and whitespace, it is
//! non-empty and carries no `\0`. A value with inner `/` (a stored URL such as
//! `/fr/produits/chaise`) contributes its LAST segment: the index stores one
//! segment per node, and the ancestors' segments come from the ancestors.

use crate::localized_name::config::NameConfig;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::translations::LocaleOverlay;
use std::collections::BTreeMap;

/// The reserved overlay pointer holding a node's translated name.
pub const NODE_NAME_POINTER: &str = "/__node_name";

/// A node's segment in one locale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameIn {
    /// Its own translated name in that locale.
    Name(String),
    /// Hidden in that locale: no segment, and not visible there.
    Hidden,
    /// No translated name: its segment there is its canonical name.
    None,
}

/// Normalize a candidate value into a node name (see the module doc).
pub fn normalize_node_name(value: &str) -> Option<String> {
    let trimmed = value.trim().trim_matches('/').trim();
    let last = trimmed.rsplit('/').next().unwrap_or(trimmed).trim();
    (!last.is_empty() && !last.contains('\0')).then(|| last.to_string())
}

/// A node's segment in `locale`, given its overlay there (`None`: no live
/// overlay). The default language always answers [`NameIn::None`].
pub fn node_name_in(overlay: Option<&LocaleOverlay>, locale: &str, cfg: &NameConfig) -> NameIn {
    if locale == cfg.default_language {
        return NameIn::None;
    }
    overlay.map_or(NameIn::None, overlay_node_name)
}

/// What one overlay says about its node's segment, BEFORE the
/// default-language rule of [`node_name_in`] (which needs the repository
/// configuration). Callers without the configuration — a transaction
/// recording which names its staged overlays set (plan Phase 13c) — use this
/// as a hint and decide through [`node_name_in`].
pub fn overlay_node_name(overlay: &LocaleOverlay) -> NameIn {
    match overlay {
        LocaleOverlay::Hidden => NameIn::Hidden,
        LocaleOverlay::Properties { data } => data
            .iter()
            .find(|(pointer, _)| pointer.as_str() == NODE_NAME_POINTER)
            .and_then(|(_, value)| match value {
                PropertyValue::String(s) => normalize_node_name(s),
                _ => None,
            })
            .map_or(NameIn::None, NameIn::Name),
    }
}

/// Every locale in which a node has a translated name of its own, from its
/// LIVE overlays by locale (the default language never has one).
pub fn localized_node_names(
    overlays: &BTreeMap<String, LocaleOverlay>,
    cfg: &NameConfig,
) -> BTreeMap<String, String> {
    overlays
        .iter()
        .filter_map(
            |(locale, overlay)| match node_name_in(Some(overlay), locale, cfg) {
                NameIn::Name(name) => Some((locale.clone(), name)),
                _ => None,
            },
        )
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_context::RepositoryConfig;
    use raisin_models::translations::JsonPointer;
    use std::collections::HashMap;

    fn cfg() -> NameConfig {
        NameConfig::from_repository(&RepositoryConfig {
            supported_languages: vec!["en".into(), "fr".into(), "de".into()],
            ..RepositoryConfig::default()
        })
    }

    fn name_overlay(pointer: &str, value: &str) -> LocaleOverlay {
        let mut data = HashMap::new();
        data.insert(
            JsonPointer::new(pointer),
            PropertyValue::String(value.into()),
        );
        LocaleOverlay::properties(data)
    }

    #[test]
    fn the_overlay_names_the_node_and_hidden_hides() {
        let cfg = cfg();
        let mut overlays = BTreeMap::new();
        overlays.insert("fr".to_string(), name_overlay(NODE_NAME_POINTER, "siege"));
        overlays.insert(
            "de".to_string(),
            name_overlay(NODE_NAME_POINTER, "/de/moebel/stuhl/"),
        );
        overlays.insert("en".to_string(), name_overlay(NODE_NAME_POINTER, "chair"));
        let names = localized_node_names(&overlays, &cfg);
        assert_eq!(names.get("fr").map(String::as_str), Some("siege"));
        assert_eq!(names.get("de").map(String::as_str), Some("stuhl"));
        assert!(
            !names.contains_key("en"),
            "the default language has no localized name"
        );

        overlays.insert("de".to_string(), LocaleOverlay::Hidden);
        assert_eq!(node_name_in(overlays.get("de"), "de", &cfg), NameIn::Hidden);
        assert!(!localized_node_names(&overlays, &cfg).contains_key("de"));
    }

    #[test]
    fn other_pointers_and_bad_values_are_not_names() {
        let cfg = cfg();
        let mut overlays = BTreeMap::new();
        overlays.insert("fr".to_string(), name_overlay("/url_fr", "chaise"));
        assert!(localized_node_names(&overlays, &cfg).is_empty());
        assert_eq!(normalize_node_name("  /  "), None);
        assert_eq!(normalize_node_name("a\0b"), None);
        assert_eq!(normalize_node_name("chaise").as_deref(), Some("chaise"));
    }
}
