// SPDX-License-Identifier: BSL-1.1

//! Whether a config change reaches items that are ALREADY synced, and what has
//! to be run if it does not.
//!
//! An ordinary sync skips an item whose etag is unchanged without even calling
//! the mapper (`batch.rs::can_skip_unmapped`), and a delta feed only reports
//! what changed upstream — which for a config change is nothing. So a setting
//! that decides what a materialized node LOOKS LIKE reaches existing items only
//! when something re-materializes them, and a setting that decides which items
//! are materialized AT ALL reaches them only when something re-enumerates the
//! provider. Neither happens on its own, ever.
//!
//! This is advisory and the endpoint acts on none of it. A remap re-materializes
//! every item on the mount and writes a node revision each — the exact cost the
//! etag skip exists to avoid — so it is the operator's call, not a side effect
//! of saving a form.

use serde::Serialize;

/// Fields whose value the MATERIALIZER reads while building a node, so a change
/// to one is invisible on already-synced items until they are re-materialized.
///
/// `path_template` is resolved per item at materialization time (`full.rs`,
/// `delta.rs`), and an unchanged item never reaches that code — which is why an
/// edited template silently leaves every existing node at its old path.
const NEEDS_REMAP: &[&str] = &["path_template"];

/// Fields that decide WHICH provider items become nodes at all.
///
/// Applied as an item filter on both walk paths. A newly-included item is only
/// re-offered by a full enumeration (the delta feed has already moved past it),
/// and a newly-excluded node is only removed by the full walk's reconcile pass.
/// A remap would also do it — a remap IS a full walk — but a plain full sync is
/// the cheaper correct answer, because these do not change the SHAPE of anything
/// that stays.
const NEEDS_FULL: &[&str] = &["include_patterns", "exclude_patterns"];

/// What the operator should run for a change to reach existing items.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub(crate) struct FollowUp {
    /// The `sync` mode to enqueue: `"remap"` or `"full"`.
    pub action: &'static str,
    /// Which of the changed fields is asking for it.
    pub fields: Vec<String>,
    /// Operator-facing sentence, so the console does not keep a second copy of
    /// this reasoning that can drift from the rule above it.
    pub reason: &'static str,
}

/// Which follow-up, if any, the changed fields imply.
///
/// Remap wins over full when both are implicated: a remap is a full walk with
/// `force_rewrite` set (`phases.rs` — "a remap is a full walk by definition"),
/// so it is a strict superset and running both would walk the mount twice.
pub(crate) fn for_changes(changed: &[String]) -> Option<FollowUp> {
    let hits = |set: &[&str]| -> Vec<String> {
        changed
            .iter()
            .filter(|c| set.contains(&c.as_str()))
            .cloned()
            .collect()
    };

    let remap = hits(NEEDS_REMAP);
    if !remap.is_empty() {
        return Some(FollowUp {
            action: "remap",
            fields: remap,
            reason: "Already-synced items keep their current shape: an ordinary sync skips an \
                     item whose etag has not changed, so it never re-runs the mapper. Remap \
                     re-materializes every item through the current mapping and hierarchy, \
                     updating nodes in place — ids, history and local additions are kept. It \
                     writes a revision per item, which is the cost the etag skip normally avoids",
        });
    }

    let full = hits(NEEDS_FULL);
    if !full.is_empty() {
        return Some(FollowUp {
            action: "full",
            fields: full,
            reason: "The delta feed only reports what changed at the provider, so a newly \
                     included item is never re-offered and a newly excluded node is never \
                     removed. A full sync re-enumerates the mount and reconciles it against \
                     the current filters",
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheduling_and_lifetime_settings_need_nothing() {
        // These are read at the START of each run, against the mount as it then
        // stands — the eviction and expiry passes walk the existing index, so
        // they reach old items without any re-materialization.
        for key in [
            "mode",
            "interval_seconds",
            "cache_content",
            "content_ttl_seconds",
            "ephemeral",
            "ttl_seconds",
            "reconcile_deletes",
            "allow_empty_reconcile",
            "max_items_per_sync",
            "max_item_failures",
            "folder_node_types",
        ] {
            assert_eq!(for_changes(&[key.to_string()]), None, "{key}");
        }
    }

    #[test]
    fn a_path_template_change_asks_for_a_remap() {
        let f = for_changes(&["path_template".to_string()]).unwrap();
        assert_eq!(f.action, "remap");
        assert_eq!(f.fields, vec!["path_template"]);
    }

    #[test]
    fn a_filter_change_asks_for_a_full_walk() {
        let f = for_changes(&["exclude_patterns".to_string()]).unwrap();
        assert_eq!(f.action, "full");
    }

    /// A remap IS a full walk, so asking for both would walk the mount twice.
    #[test]
    fn remap_absorbs_a_full() {
        let f =
            for_changes(&["include_patterns".to_string(), "path_template".to_string()]).unwrap();
        assert_eq!(f.action, "remap");
        assert_eq!(f.fields, vec!["path_template"]);
    }
}
