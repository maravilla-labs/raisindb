//! Type aliases and structs for transaction change tracking
//!
//! These types replace complex nested tuples used throughout the transaction
//! commit pipeline, improving readability and maintainability.

use raisin_hlc::HLC;
use raisin_models::tree::ChangeOperation;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Tracks a single node change within a transaction.
///
/// Stored per node_id in [`ChangedNodesMap`] during transaction operations.
/// Path and node_type are preserved for proper WebSocket subscription matching
/// on delete events (where the node is no longer queryable).
#[derive(Debug, Clone)]
pub struct NodeChange {
    pub workspace: String,
    pub revision: HLC,
    pub operation: ChangeOperation,
    /// Node path, stored for event matching on delete
    pub path: Option<String>,
    /// Node type, stored for subscription filtering on delete
    pub node_type: Option<String>,
    /// Top-level property names whose value differs from the node's state
    /// BEFORE this transaction (sorted, engine-internal keys excluded).
    ///
    /// `None` means "unknown" — the previous state was not available on the
    /// path that recorded the change — and is emitted as NO
    /// `changed_properties` metadata, so a trigger filtering on it fails
    /// open. Only meaningful for `Modified`.
    pub changed_properties: Option<Vec<String>>,
}

/// Maps node_id to its change info for a transaction.
pub type ChangedNodesMap = HashMap<String, NodeChange>;

/// Thread-safe shared state for tracking changed nodes during a transaction.
pub type SharedChangedNodes = Arc<Mutex<ChangedNodesMap>>;

/// Tracks a single translation change within a transaction.
#[derive(Debug, Clone)]
pub struct TranslationChange {
    pub workspace: String,
    pub revision: HLC,
    pub operation: ChangeOperation,
}

/// Maps (node_id, locale) to its translation change info.
pub type ChangedTranslationsMap = HashMap<(String, String), TranslationChange>;

/// Thread-safe shared state for tracking changed translations during a transaction.
pub type SharedChangedTranslations = Arc<Mutex<ChangedTranslationsMap>>;

/// Metadata extracted from a transaction for the commit phase.
///
/// Replaces the 7-tuple returned by `extract_commit_metadata`.
pub struct CommitMetadata {
    pub tenant_id: Arc<String>,
    pub repo_id: Arc<String>,
    pub branch: Option<Arc<String>>,
    pub transaction_revision: Option<HLC>,
    pub actor: Option<Arc<String>>,
    pub message: Option<Arc<String>>,
    pub is_system: bool,
    pub bookkeeping: bool,
}

/// Whether a property key is engine-internal (`__…`, `$mixins`, …) and so
/// never reported as a changed property.
fn is_internal_property(name: &str) -> bool {
    name.starts_with("__") || name.starts_with('$')
}

/// Top-level property names that were added, removed or changed between
/// `old` and `new`, sorted. Values are compared as serde_json values, so a
/// difference that only exists in the in-memory representation is not a
/// change.
pub fn changed_property_names<V: serde::Serialize + PartialEq>(
    old: &HashMap<String, V>,
    new: &HashMap<String, V>,
) -> Vec<String> {
    let differs =
        |a: &V, b: &V| a != b && serde_json::to_value(a).ok() != serde_json::to_value(b).ok();
    let mut names: Vec<String> = new
        .iter()
        .filter(|(k, v)| old.get(*k).map_or(true, |o| differs(o, v)))
        .map(|(k, _)| k.clone())
        .chain(old.keys().filter(|k| !new.contains_key(*k)).cloned())
        .filter(|k| !is_internal_property(k))
        .collect();
    names.sort();
    names.dedup();
    names
}

/// Union two known changed-property lists; unknown on either side stays
/// unknown (a later write in the same transaction cannot make the earlier
/// write's changes known).
pub fn merge_changed_properties(
    earlier: Option<&Vec<String>>,
    later: Vec<String>,
) -> Option<Vec<String>> {
    let mut merged = earlier?.clone();
    merged.extend(later);
    merged.sort();
    merged.dedup();
    Some(merged)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn props(v: serde_json::Value) -> HashMap<String, serde_json::Value> {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn changed_property_names_reports_added_removed_and_changed() {
        let old = props(json!({"title": "a", "body": "x", "gone": 1, "same": [1, 2]}));
        let new = props(json!({"title": "b", "body": "x", "added": true, "same": [1, 2]}));
        assert_eq!(
            changed_property_names(&old, &new),
            vec!["added".to_string(), "gone".to_string(), "title".to_string()]
        );
    }

    #[test]
    fn changed_property_names_ignores_internal_keys() {
        let old = props(json!({"__stamp": 1, "$mixins": ["a"], "title": "a"}));
        let new = props(json!({"__stamp": 2, "$supertypes": ["b"], "title": "a"}));
        assert!(changed_property_names(&old, &new).is_empty());
    }

    #[test]
    fn changed_property_names_compares_nested_values() {
        let old = props(json!({"address": {"city": "Bern"}}));
        let new = props(json!({"address": {"city": "Basel"}}));
        assert_eq!(changed_property_names(&old, &new), vec!["address"]);
    }

    #[test]
    fn merge_changed_properties_unions_and_keeps_unknown() {
        let earlier = vec!["title".to_string()];
        assert_eq!(
            merge_changed_properties(Some(&earlier), vec!["body".into(), "title".into()]),
            Some(vec!["body".to_string(), "title".to_string()])
        );
        assert_eq!(merge_changed_properties(None, vec!["body".into()]), None);
    }
}
