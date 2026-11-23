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
