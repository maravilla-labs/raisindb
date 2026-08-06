//! Which fields travel, and what a move means for this mount.
//!
//! Split from [`super::plan`] because it answers a different question. `plan`
//! decides WHETHER a mount writes at all — the mode, the adapter shortfall, the
//! refusal — and this decides what goes on the wire once it does. Keeping the
//! move split here also keeps it in ONE place: `state_only` and `mirror` share
//! it verbatim, and a second copy is how the two modes would drift apart.

use super::super::Capabilities;
use super::policy::MovePolicy;

/// The effective field allow-list, split by what a move means for this mount.
///
/// One type shared by `state_only` and `mirror` because the split is identical
/// in both: a move is an `update` carrying a declared location field, so
/// `move_policy` is a rule about WHICH FIELDS TRAVEL and nothing else. There is
/// no `move` operation and no second code path.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FieldPlan {
    /// What actually goes on the wire. MAY be empty: a mirror whose value is
    /// delete propagation (a file mount where nothing about the node maps back
    /// to a writable provider field) is a legitimate configuration, and
    /// refusing it would force an operator to invent a mutable field to get
    /// deletes. Under a non-`push` move policy it is also what is left after
    /// the location fields are withheld.
    pub push: Vec<String>,
    /// The effective fields the adapter declared as relocating the object
    /// ([`Capabilities::move_fields`] ∩ the effective list). Empty for every
    /// adapter that declares none, which is all of them today — and an empty
    /// list makes every policy behave identically, so nothing changes for a
    /// mount whose adapter has not opted in.
    pub moves: Vec<String>,
    pub policy: MovePolicy,
}

impl FieldPlan {
    /// Split the effective allow-list under a resolved policy.
    ///
    /// `push` keeps the effective order (which is the mount's declared order),
    /// so the field list an adapter receives is stable run to run.
    pub(super) fn new(
        effective: Vec<String>,
        capabilities: &Capabilities,
        policy: MovePolicy,
    ) -> Self {
        let moves: Vec<String> = effective
            .iter()
            .filter(|f| capabilities.move_fields.iter().any(|m| m == *f))
            .cloned()
            .collect();
        let push = if policy.pushes() || moves.is_empty() {
            effective
        } else {
            // Withheld rather than sent-and-ignored. A field dropped here is
            // also dropped from the converge check and from `__pushed_state`,
            // which is what stops a withheld move from re-nominating the node
            // on every drain forever.
            effective
                .into_iter()
                .filter(|f| !moves.contains(f))
                .collect()
        };
        Self {
            push,
            moves,
            policy,
        }
    }

    /// A plan that pushes exactly these fields and knows nothing about moves —
    /// the shape every mount has until its adapter declares `move_fields`.
    #[cfg(test)]
    pub(crate) fn pushing(fields: &[&str]) -> Self {
        Self {
            push: fields.iter().map(|s| s.to_string()).collect(),
            moves: Vec::new(),
            policy: MovePolicy::Detach,
        }
    }

    /// Whether this node's move fields diverge from what was last pushed, i.e.
    /// whether the user moved it since the provider last agreed.
    pub(super) fn moved(&self, view: &super::super::materializer::WriteView) -> bool {
        !self.moves.is_empty() && view.diverges(&self.moves)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The move split: `push` carries the location field, `detach` and `reject`
    /// withhold it, and every policy leaves the ordinary fields alone.
    #[test]
    fn the_move_policy_decides_only_whether_the_location_field_travels() {
        let caps = Capabilities {
            move_fields: vec!["folder".to_string()],
            ..Default::default()
        };
        let effective = || vec!["unread".to_string(), "folder".to_string()];

        let pushed = FieldPlan::new(effective(), &caps, MovePolicy::Push);
        assert_eq!(pushed.push, vec!["unread", "folder"]);
        assert_eq!(pushed.moves, vec!["folder"]);

        for policy in [MovePolicy::Detach, MovePolicy::Reject] {
            let plan = FieldPlan::new(effective(), &caps, policy);
            assert_eq!(plan.push, vec!["unread"], "{policy:?} must withhold folder");
            assert_eq!(plan.moves, vec!["folder"]);
        }

        // An adapter that declares no move fields behaves identically under
        // every policy — which is what makes this backward compatible.
        let none = Capabilities::default();
        for policy in [MovePolicy::Push, MovePolicy::Detach, MovePolicy::Reject] {
            let plan = FieldPlan::new(effective(), &none, policy);
            assert_eq!(plan.push, vec!["unread", "folder"]);
            assert!(plan.moves.is_empty());
        }
    }
}
