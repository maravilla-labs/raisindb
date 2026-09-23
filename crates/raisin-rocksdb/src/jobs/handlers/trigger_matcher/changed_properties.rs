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

//! The `filters.changed_properties` trigger filter.
//!
//! `changed_properties: ["title", ...]` narrows an `Updated` event to writes
//! that changed at least one of the listed top-level properties. The commit
//! stamps the changed names on the event (`metadata.changed_properties`).
//!
//! Two deliberate fail-open rules:
//! - Created / Deleted (anything but `Updated`) are never narrowed by it.
//! - An `Updated` event WITHOUT the metadata passes: an emitter that does not
//!   know the previous state must not silently stop a trigger.

use super::inline_triggers::InlineTriggerContext;
use crate::jobs::handlers::FilterCheckResult;

/// Evaluate the `changed_properties` filter from a trigger's `filters` JSON,
/// recording a [`FilterCheckResult`] when the filter is declared.
///
/// Returns `true` if the filter passes (or is not declared).
pub(super) fn changed_properties_filter(
    filters: &serde_json::Value,
    ctx: &InlineTriggerContext<'_>,
    filter_checks: &mut Vec<FilterCheckResult>,
) -> bool {
    let Some(wanted) = filters.get("changed_properties").and_then(|v| v.as_array()) else {
        return true;
    };
    let check = check_changed_properties(wanted, ctx.event_type, ctx.changed_properties);
    let passed = check.passed;
    filter_checks.push(check);
    passed
}

/// Pure filter semantics, see the module docs.
pub(super) fn check_changed_properties(
    wanted: &[serde_json::Value],
    event_type: &str,
    changed: Option<&[String]>,
) -> FilterCheckResult {
    let expected = Some(serde_json::json!(wanted));
    let actual = changed.map(|c| serde_json::json!(c));
    let (passed, reason) = if event_type != "Updated" {
        (
            true,
            format!("{event_type} events are not narrowed by changed_properties"),
        )
    } else {
        match changed {
            None => (
                true,
                "Event carries no changed_properties metadata; filter fails open".to_string(),
            ),
            Some(changed) => {
                let hit = wanted
                    .iter()
                    .filter_map(|w| w.as_str())
                    .find(|w| changed.iter().any(|c| c == w));
                match hit {
                    Some(name) => (true, format!("Property {name} changed")),
                    None => (
                        false,
                        format!("None of the listed properties changed (changed: {changed:?})"),
                    ),
                }
            }
        }
    };
    FilterCheckResult {
        filter_name: "changed_properties".to_string(),
        passed,
        expected,
        actual,
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn wanted() -> Vec<serde_json::Value> {
        vec![json!("title"), json!("status")]
    }

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn changed_properties_matches_when_any_listed_name_changed() {
        let changed = names(&["body", "status"]);
        let r = check_changed_properties(&wanted(), "Updated", Some(&changed));
        assert!(r.passed, "{}", r.reason);
        assert_eq!(r.filter_name, "changed_properties");
    }

    #[test]
    fn changed_properties_rejects_when_no_listed_name_changed() {
        let changed = names(&["body"]);
        let r = check_changed_properties(&wanted(), "Updated", Some(&changed));
        assert!(!r.passed);
        let empty: Vec<String> = Vec::new();
        assert!(!check_changed_properties(&wanted(), "Updated", Some(&empty)).passed);
    }

    #[test]
    fn changed_properties_missing_metadata_passes() {
        assert!(check_changed_properties(&wanted(), "Updated", None).passed);
    }

    #[test]
    fn changed_properties_does_not_affect_created_or_deleted() {
        let changed = names(&["body"]);
        assert!(check_changed_properties(&wanted(), "Created", Some(&changed)).passed);
        assert!(check_changed_properties(&wanted(), "Created", None).passed);
        assert!(check_changed_properties(&wanted(), "Deleted", None).passed);
    }

    #[test]
    fn changed_properties_filter_reads_trigger_filters_json() {
        let changed = names(&["title"]);
        let ctx = InlineTriggerContext {
            event_type: "Updated",
            node_id: "n",
            node_type: "t:T",
            node_path: "/n",
            tenant_id: "t",
            repo_id: "r",
            branch: "main",
            workspace: "ws",
            node_properties: None,
            changed_properties: Some(&changed),
        };
        let mut checks = Vec::new();
        let filters = json!({"changed_properties": ["title"]});
        assert!(changed_properties_filter(&filters, &ctx, &mut checks));
        assert_eq!(checks.len(), 1);

        let mut checks = Vec::new();
        let filters = json!({"changed_properties": ["price"]});
        assert!(!changed_properties_filter(&filters, &ctx, &mut checks));

        // Undeclared filter: passes and records nothing.
        let mut checks = Vec::new();
        assert!(changed_properties_filter(&json!({}), &ctx, &mut checks));
        assert!(checks.is_empty());
    }
}
