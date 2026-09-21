//! Registration-time check of a trigger's `event_kinds`.
//!
//! A node event reaches trigger evaluation as a PascalCase kind string
//! (`jobs/event_handler`: "Created" / "Updated" from `handle_local_node_change`,
//! "Deleted" from the delete handler), and the registry and the matcher compare
//! it EXACTLY. So `event_kinds: ['updated']`, `['Update']` or `['Published']`
//! registers without complaint and produces a trigger that can never fire —
//! a silent failure that looks like a broken function.
//!
//! This is a WARNING, not a refusal: triggers already stored with such a kind
//! must keep loading, and a kind that is not delivered today may be tomorrow.

/// The kinds the node event handler delivers to `node_event` triggers today.
pub(crate) const DELIVERED_EVENT_KINDS: [&str; 3] = ["Created", "Updated", "Deleted"];

/// Every `NodeEventKind` name (raisin-events and the functions trigger config
/// together), in the spelling the registry compares against.
pub(crate) const NODE_EVENT_KIND_NAMES: [&str; 8] = [
    "Created",
    "Updated",
    "Deleted",
    "Published",
    "Unpublished",
    "Moved",
    "Renamed",
    "Reordered",
];

/// One message per kind in `kinds` that can never match a delivered event.
/// Empty when every kind is one the handler delivers.
pub(crate) fn event_kind_problems(kinds: &[String]) -> Vec<String> {
    kinds
        .iter()
        .filter(|kind| !DELIVERED_EVENT_KINDS.contains(&kind.as_str()))
        .map(|kind| {
            if let Some(canonical) = NODE_EVENT_KIND_NAMES
                .iter()
                .find(|name| name.eq_ignore_ascii_case(kind))
            {
                if *canonical != kind.as_str() {
                    return format!(
                        "event kind '{kind}' never matches: kinds compare exactly, write '{canonical}' \
                         (delivered kinds: {})",
                        DELIVERED_EVENT_KINDS.join(", ")
                    );
                }
                return format!(
                    "event kind '{kind}' is a NodeEventKind but is not delivered to node_event \
                     triggers, so it never fires (delivered kinds: {})",
                    DELIVERED_EVENT_KINDS.join(", ")
                );
            }
            format!(
                "unknown event kind '{kind}' never matches; valid NodeEventKind names: {} \
                 (delivered to node_event triggers: {})",
                NODE_EVENT_KIND_NAMES.join(", "),
                DELIVERED_EVENT_KINDS.join(", ")
            )
        })
        .collect()
}

/// Log a WARN for each kind of this trigger that can never fire. The trigger
/// is still registered.
pub(super) fn warn_unmatchable_event_kinds(trigger: &str, kinds: &[String]) {
    for problem in event_kind_problems(kinds) {
        tracing::warn!(trigger = %trigger, "Trigger registered with {}", problem);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(k: &[&str]) -> Vec<String> {
        k.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn delivered_kinds_are_quiet() {
        assert!(event_kind_problems(&kinds(&["Created", "Updated", "Deleted"])).is_empty());
    }

    #[test]
    fn an_unknown_kind_is_named_with_the_valid_names() {
        let problems = event_kind_problems(&kinds(&["Created", "Changed"]));
        assert_eq!(problems.len(), 1);
        assert!(
            problems[0].contains("unknown event kind 'Changed'"),
            "{}",
            problems[0]
        );
        assert!(
            problems[0].contains("Created, Updated, Deleted, Published"),
            "{}",
            problems[0]
        );
    }

    #[test]
    fn a_wrong_case_kind_names_the_right_spelling() {
        let problems = event_kind_problems(&kinds(&["updated"]));
        assert!(problems[0].contains("write 'Updated'"), "{}", problems[0]);
    }

    #[test]
    fn a_real_but_undelivered_kind_is_called_out() {
        let problems = event_kind_problems(&kinds(&["Published"]));
        assert!(problems[0].contains("not delivered"), "{}", problems[0]);
    }
}
