// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Comparing and merging a built-in NodeType with the copy stored on a branch.
//!
//! # Why the stored copy, not the applied hash
//!
//! The applied-hash record is keyed by tenant and repository with no branch,
//! while NodeTypes are stored per branch. The startup resync visits branches in
//! key order, so the first branch it reaches records the hash, and every later
//! branch that already held an OLDER copy matched on hash and was skipped. The
//! pending-updates check trusted the same hash and reported nothing. That left
//! `raisin:AclFolder` on a tenant's `main` without `raisin:ConversationRecord`
//! (and three earlier additions), and every chat message failed with
//! "not allowed as child of 'raisin:AclFolder'" while system-updates said
//! "0 pending".
//!
//! So whether a branch is up to date is decided by its stored definition:
//! [`stored_nodetype_matches`] is true only when the stored copy already
//! carries everything the built-in definition says.
//!
//! # Why merge
//!
//! A stored built-in may carry relations the YAML does not: an allowed child or
//! mixin a package or the tenant added. Writing the YAML over it would drop
//! them, and because a removal classifies as breaking it would also park the
//! type as pending forever. [`merge_with_stored`] keeps those entries — the same
//! reasoning as the workspace allow-list merge in `resync.rs` — so a built-in
//! update never takes an allowed child away.

use raisin_models::nodes::types::NodeType;

fn union(from_definition: &[String], from_storage: &[String]) -> Vec<String> {
    let mut merged = from_definition.to_vec();
    for existing in from_storage {
        if !merged.contains(existing) {
            merged.push(existing.clone());
        }
    }
    merged
}

/// The definition that should be written over `stored`: the built-in
/// definition, plus every allowed child and mixin the stored copy has and the
/// definition does not. Publication state is kept.
///
/// Properties are NOT merged: dropping a property stays a breaking change that
/// an operator applies deliberately (forced apply), as before.
pub fn merge_with_stored(definition: &NodeType, stored: &NodeType) -> NodeType {
    let mut merged = definition.clone();
    merged.allowed_children = union(&definition.allowed_children, &stored.allowed_children);
    merged.mixins = union(&definition.mixins, &stored.mixins);

    merged.published_at = stored.published_at;
    merged.published_by = stored.published_by.clone();
    merged
}

/// The part of `definition` that can be written over `stored` without a
/// breaking change: [`merge_with_stored`], except that a property the stored
/// copy already declares keeps its stored type, and keeps being optional when
/// the stored copy says so, and a property only the stored copy has is kept.
///
/// A definition whose full apply is breaking (a type corrected from `String`
/// to `Date`, a property dropped) used to be withheld WHOLE, so a branch also
/// went without every additive change that came with or after it: a forked
/// `publish` or `edit-*` branch kept a `raisin:Package` without
/// `content_hash`, a `raisin:Event` without `start_utc`, years of additions
/// behind one old type change. The resync writes this instead and leaves only
/// the breaking remainder pending for an operator.
pub fn non_breaking_merge(definition: &NodeType, stored: &NodeType) -> NodeType {
    let mut merged = merge_with_stored(definition, stored);
    let stored_props = stored.properties.clone().unwrap_or_default();
    let mut props = Vec::new();
    for prop in definition.properties.clone().unwrap_or_default() {
        let mut prop = prop;
        if let Some(old) = stored_props
            .iter()
            .find(|p| p.name.is_some() && p.name == prop.name)
        {
            if old.property_type != prop.property_type {
                prop.property_type = old.property_type.clone();
            }
            if !old.required.unwrap_or(false) && prop.required.unwrap_or(false) {
                prop.required = old.required;
            }
        }
        props.push(prop);
    }
    for old in &stored_props {
        let declared = props.iter().any(|p| p.name.is_some() && p.name == old.name);
        if !declared {
            props.push(old.clone());
        }
    }
    merged.properties = if props.is_empty() && definition.properties.is_none() {
        None
    } else {
        Some(props)
    };
    merged
}

/// Whether two copies of a NodeType say the same thing (ignoring what storage
/// owns: identity, timestamps, the stored version).
pub fn same_definition(a: &NodeType, b: &NodeType) -> bool {
    normalized(a) == normalized(b)
}

/// Strip what storage owns (identity, timestamps, the stored `version`, which
/// storage bumps on every write) and order-insensitive lists, so two copies of
/// the same definition compare equal.
fn normalized(nt: &NodeType) -> NodeType {
    let mut n = nt.clone();
    n.id = None;
    n.version = None;
    n.created_at = None;
    n.updated_at = None;
    n.published_at = None;
    n.published_by = None;
    n.previous_version = None;
    n.allowed_children.sort();
    n.allowed_children.dedup();
    n.mixins.sort();
    n.mixins.dedup();
    n
}

/// True when `stored` already carries everything `definition` declares
/// (entries only the stored copy has are fine — they are kept, not drift).
pub fn stored_nodetype_matches(definition: &NodeType, stored: &NodeType) -> bool {
    normalized(&merge_with_stored(definition, stored)) == normalized(stored)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nt(yaml: &str) -> NodeType {
        serde_yaml::from_str(yaml).expect("test NodeType YAML should parse")
    }

    #[test]
    fn the_non_breaking_part_adds_and_keeps_but_never_changes_a_type() {
        let def = nt(
            "name: t:E\nproperties:\n  - name: start\n    type: Date\n    required: true\n  - name: start_utc\n    type: String\n",
        );
        let stored = nt(
            "name: t:E\nproperties:\n  - name: start\n    type: String\n  - name: legacy\n    type: String\n",
        );
        let merged = non_breaking_merge(&def, &stored);
        let props = merged.properties.clone().unwrap();
        let by = |n: &str| props.iter().find(|p| p.name.as_deref() == Some(n)).cloned();
        assert_eq!(
            by("start").unwrap().property_type,
            by_type(&stored, "start")
        );
        assert_ne!(
            by("start").unwrap().required,
            Some(true),
            "not made required"
        );
        assert!(by("start_utc").is_some(), "the new property lands");
        assert!(by("legacy").is_some(), "a stored-only property is kept");
        assert!(
            crate::system_updates::detect_nodetype_breaking_changes(&stored, &merged).is_empty(),
            "nothing breaking is written"
        );
        assert!(
            !stored_nodetype_matches(&def, &merged),
            "the breaking remainder stays pending"
        );
        assert!(
            same_definition(&non_breaking_merge(&def, &merged), &merged),
            "idempotent"
        );
    }

    fn by_type(
        nt: &NodeType,
        name: &str,
    ) -> raisin_models::nodes::properties::schema::PropertyType {
        nt.properties
            .as_ref()
            .unwrap()
            .iter()
            .find(|p| p.name.as_deref() == Some(name))
            .unwrap()
            .property_type
            .clone()
    }

    #[test]
    fn missing_allowed_child_is_drift() {
        let def = nt("name: t:F\nversion: 5\nallowed_children: [a:A, a:B]\n");
        let stored = nt("name: t:F\nversion: 3\nallowed_children: [a:A]\n");
        assert!(!stored_nodetype_matches(&def, &stored));
        assert_eq!(
            merge_with_stored(&def, &stored).allowed_children,
            vec!["a:A", "a:B"]
        );
    }

    #[test]
    fn stored_extra_relations_are_kept_and_are_not_drift() {
        let def =
            nt("name: t:F\nallowed_children: [a:A]\nproperties:\n  - name: x\n    type: String\n");
        let stored = nt(
            "name: t:F\nversion: 12\nallowed_children: [custom:C, a:A]\nmixins: [m:M]\nproperties:\n  - name: x\n    type: String\n",
        );
        assert!(stored_nodetype_matches(&def, &stored));
        let merged = merge_with_stored(&def, &stored);
        assert_eq!(merged.allowed_children, vec!["a:A", "custom:C"]);
        assert_eq!(merged.mixins, vec!["m:M"]);
    }

    #[test]
    fn a_stored_only_property_is_drift_not_merged() {
        // Removing a property stays the operator's (breaking) decision.
        let def = nt("name: t:F\nproperties:\n  - name: x\n    type: String\n");
        let stored = nt("name: t:F\nproperties:\n  - name: x\n    type: String\n  - name: old\n    type: String\n");
        assert!(!stored_nodetype_matches(&def, &stored));
    }

    #[test]
    fn missing_property_is_drift() {
        let def = nt("name: t:F\nproperties:\n  - name: x\n    type: String\n  - name: y\n    type: String\n");
        let stored = nt("name: t:F\nproperties:\n  - name: x\n    type: String\n");
        assert!(!stored_nodetype_matches(&def, &stored));
    }

    #[test]
    fn stored_version_and_identity_do_not_count() {
        let def = nt("name: t:F\nversion: 5\nallowed_children: [a:A]\n");
        let mut stored = nt("name: t:F\nversion: 9\nallowed_children: [a:A]\n");
        stored.id = Some("x".into());
        stored.created_at = Some(chrono::Utc::now());
        assert!(stored_nodetype_matches(&def, &stored));
    }
}
