//! Which of a mount's nodes still need pushing.

use super::super::materializer::VirtualNodeRef;
use super::fields::FieldPlan;
use super::policy::MovePolicy;

/// One node the drain will act on.
#[derive(Debug, Clone)]
pub(super) struct Candidate {
    pub node_id: String,
    pub external_id: String,
}

/// Nodes whose watched fields no longer match what was last pushed.
///
/// A node that diverges ONLY on a withheld move field is nominated under
/// `move_policy: reject` and not under `detach`. That asymmetry is the whole
/// difference between the two policies: `detach` is a decision, so it is silent
/// and costs nothing; `reject` is a refusal, and a refusal nobody is told about
/// is indistinguishable from a mount that quietly stopped writing. The
/// nomination costs one node read per drain and zero provider calls — the same
/// price a `gone` pays for the same reason.
///
/// Derived from the index the run already loaded, so detection costs no reads
/// at all. It is allowed to be imprecise in the noisy direction: every
/// candidate is re-read and re-checked before anything is sent (see
/// [`super::push`]), which is the same division of labour the read path uses
/// between a sloppy delta feed and the exact etag skip-write.
pub(super) fn candidates(
    nodes: Vec<VirtualNodeRef>,
    plan: &FieldPlan,
    limit: usize,
) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = nodes
        .into_iter()
        .filter_map(|node| {
            // A derived recurrence occurrence is NEVER a write candidate. The
            // projection regenerates it from its series master on every rebuild,
            // so anything pushed from one is discarded locally at the next tick
            // while the provider keeps the change — a divergence with no error
            // anywhere. Filtered here rather than only at push time so it cannot
            // consume the run's item budget either; `push::push_one` repeats the
            // check against the node's own properties, for a projection node
            // that somehow landed outside the projection root.
            if let Some(reason) = crate::jobs::handlers::calendar_expand::path_refusal(&node.path) {
                tracing::warn!(
                    path = %node.path,
                    external_id = %node.external_id,
                    "{reason}"
                );
                return None;
            }
            // An unseeded node (no `__pushed_state`) needs no special arm: with
            // nothing recorded as pushed, any watched field it carries already
            // counts as diverged, and it is PUSHED rather than baselined from
            // its own local values — see [`super::push::push_one`] for why
            // baselining is the one outcome that can silently lose an edit.
            let view = node.write_view.as_ref()?;
            let nominate = view.diverges(&plan.push)
                || (plan.policy == MovePolicy::Reject && plan.moved(view));
            nominate.then_some(Candidate {
                node_id: node.id,
                external_id: node.external_id,
            })
        })
        .collect();
    // The index is a hash map, so without this the truncation below would drop
    // a different arbitrary subset on every run and a mount with more pending
    // pushes than the cap could starve one of them indefinitely.
    out.sort_by(|a, b| a.external_id.cmp(&b.external_id));
    out.truncate(limit);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::handlers::virtual_mount_sync::materializer::WriteView;
    use crate::jobs::handlers::virtual_mount_sync::Capabilities;
    use serde_json::{json, Map};

    fn diverged_ref(id: &str, path: &str) -> VirtualNodeRef {
        let mut watched = Map::new();
        watched.insert("unread".into(), json!(true));
        VirtualNodeRef {
            id: id.to_string(),
            path: path.to_string(),
            external_id: id.to_string(),
            is_command: false,
            etag: None,
            synced_secs: None,
            // No `pushed` at all, so every watched field counts as diverged —
            // i.e. this node WOULD be a candidate if nothing filtered it.
            write_view: Some(Box::new(WriteView {
                watched,
                pushed: None,
            })),
            pushed_state: None,
        }
    }

    /// A plan that pushes exactly the named fields and knows nothing about
    /// moves — the shape every mount had before `move_fields` existed.
    fn plain(fields: &[&str]) -> FieldPlan {
        FieldPlan {
            push: fields.iter().map(|s| s.to_string()).collect(),
            moves: Vec::new(),
            policy: MovePolicy::Detach,
        }
    }

    /// A derived recurrence occurrence must never be nominated for a push. The
    /// projection regenerates it from its series master, so anything pushed
    /// from one is discarded locally at the next rebuild while the provider
    /// keeps the change — divergence with no error anywhere.
    #[test]
    fn a_derived_occurrence_is_never_a_write_candidate() {
        let fields = plain(&["unread"]);
        let nodes = vec![
            diverged_ref("real", "/mail/inbox/real"),
            diverged_ref("occ", "/_occurrences/m1/2026/03/20260331T070000Z"),
        ];
        let out = candidates(nodes, &fields, 10);
        assert_eq!(out.len(), 1, "only the ordinary node is a candidate");
        assert_eq!(out[0].node_id, "real");
    }

    /// The filter is a path-boundary check, not a prefix check: an ordinary
    /// mount whose path merely starts with the projection root's characters is
    /// still writable.
    #[test]
    fn a_sibling_sharing_the_prefix_is_still_a_candidate() {
        let nodes = vec![diverged_ref("sib", "/_occurrences_archive/thing")];
        assert_eq!(candidates(nodes, &plain(&["unread"]), 10).len(), 1);
    }

    /// A node that diverges ONLY on a withheld move field is nominated under
    /// `reject` (so the refusal is stated) and not under `detach` (which is a
    /// decision, and a decision does not need re-announcing every drain).
    #[test]
    fn only_reject_nominates_a_node_that_moved_and_nothing_else() {
        let mut node = diverged_ref("m1", "/mail/inbox/m1");
        // Watched: a folder that moved, and an `unread` that matches what was
        // pushed — so the ONLY divergence is the move.
        let view = node.write_view.as_mut().unwrap();
        view.watched.clear();
        view.watched.insert("unread".into(), json!(false));
        view.watched.insert("folder".into(), json!("archive"));
        let mut pushed = Map::new();
        pushed.insert("unread".into(), json!(false));
        pushed.insert("folder".into(), json!("inbox"));
        view.pushed = Some(pushed);

        let caps = Capabilities {
            move_fields: vec!["folder".to_string()],
            ..Default::default()
        };
        let effective = || vec!["unread".to_string(), "folder".to_string()];

        let detach = FieldPlan::new(effective(), &caps, MovePolicy::Detach);
        assert!(
            candidates(vec![node.clone()], &detach, 10).is_empty(),
            "a detached move must not nominate the node at all"
        );

        let reject = FieldPlan::new(effective(), &caps, MovePolicy::Reject);
        assert_eq!(
            candidates(vec![node], &reject, 10).len(),
            1,
            "a rejected move must be nominated so the refusal can be reported"
        );
    }
}
