// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Partial property updates ("patches").
//!
//! A write that reads a node, changes one property and writes the WHOLE
//! property map back loses every write of another property that commits
//! between its read and its commit. A [`PropertyPatch`] instead names only the
//! properties it sets or removes, and the storage applies it to the node's
//! state as stored when the write lands
//! ([`crate::transactional::TransactionalContext::patch_node_properties`]).
//!
//! Semantics, per property: the write that lands later wins; a property the
//! patch does not name is never touched. Callers take no lock, check no
//! version and retry nothing.

use std::collections::HashMap;

use raisin_models::nodes::properties::PropertyValue;
use serde_json::{Map, Value};

/// Which node a patch applies to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatchTarget {
    /// By node id.
    Id(String),
    /// By path.
    Path(String),
}

/// How `null` values in a JSON patch are read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NullMode {
    /// A `null` removes the property (JSON merge-patch, RFC 7386).
    Remove,
    /// A `null` is stored as a null value (`properties || {...}` in SQL).
    Store,
}

/// The top-level properties one write sets and removes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PropertyPatch {
    /// Properties to set (replacing the stored value as a whole).
    pub set: Vec<(String, PropertyValue)>,
    /// Properties to remove.
    pub remove: Vec<String>,
}

impl PropertyPatch {
    /// A patch setting `set`.
    pub fn setting(set: Vec<(String, PropertyValue)>) -> Self {
        Self {
            set,
            remove: Vec::new(),
        }
    }

    /// Read a JSON object as a patch.
    pub fn from_json(map: &Map<String, Value>, nulls: NullMode) -> Self {
        let mut patch = Self::default();
        for (key, value) in map {
            if value.is_null() && nulls == NullMode::Remove {
                patch.remove.push(key.clone());
            } else {
                patch
                    .set
                    .push((key.clone(), PropertyValue::from_json(value)));
            }
        }
        patch
    }

    /// Whether the patch names no property.
    pub fn is_empty(&self) -> bool {
        self.set.is_empty() && self.remove.is_empty()
    }

    /// The property names this patch touches.
    pub fn touched(&self) -> impl Iterator<Item = &str> {
        self.set
            .iter()
            .map(|(k, _)| k.as_str())
            .chain(self.remove.iter().map(String::as_str))
    }

    /// Apply to a property map: named properties set or removed, every other
    /// property left as it is.
    pub fn apply(&self, properties: &mut HashMap<String, PropertyValue>) {
        for key in &self.remove {
            properties.remove(key);
        }
        for (key, value) in &self.set {
            properties.insert(key.clone(), value.clone());
        }
    }

    /// `later` applied after `self` (a later patch of the same node in one
    /// transaction): the later one wins per property.
    pub fn then(&mut self, later: PropertyPatch) {
        let touched: Vec<String> = later.touched().map(str::to_string).collect();
        self.set.retain(|(k, _)| !touched.contains(k));
        self.remove.retain(|k| !touched.contains(k));
        self.set.extend(later.set);
        self.remove.extend(later.remove);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn props(v: Value) -> HashMap<String, PropertyValue> {
        v.as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), PropertyValue::from_json(v)))
            .collect()
    }

    #[test]
    fn apply_touches_only_named_properties() {
        let mut stored = props(json!({"a": "1", "b": "2", "c": "3"}));
        let patch = PropertyPatch::from_json(
            json!({"b": "x", "c": null, "d": "4"}).as_object().unwrap(),
            NullMode::Remove,
        );
        patch.apply(&mut stored);
        assert_eq!(stored, props(json!({"a": "1", "b": "x", "d": "4"})));
    }

    #[test]
    fn store_mode_keeps_null_values() {
        let patch =
            PropertyPatch::from_json(json!({"a": null}).as_object().unwrap(), NullMode::Store);
        assert!(patch.remove.is_empty());
        assert_eq!(patch.set.len(), 1);
    }

    #[test]
    fn later_patch_wins_per_property() {
        let mut first = PropertyPatch::from_json(
            json!({"a": "1", "b": "1"}).as_object().unwrap(),
            NullMode::Remove,
        );
        first.then(PropertyPatch::from_json(
            json!({"b": null, "c": "2"}).as_object().unwrap(),
            NullMode::Remove,
        ));
        let mut stored = props(json!({"b": "0", "z": "z"}));
        first.apply(&mut stored);
        assert_eq!(stored, props(json!({"a": "1", "c": "2", "z": "z"})));
    }
}
