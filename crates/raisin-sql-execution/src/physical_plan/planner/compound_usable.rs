//! Whether a MATCHED compound index may serve a query (plan Phases 13e/13f):
//! the editorial-order guard and the fail-closed build-state gate, applied to
//! each ranked candidate in turn (`rank_compound_indexes`) so an unusable best
//! match never shadows a usable one that answers as well.

use super::compound_index::CompoundIndexMatch;
use super::{PhysicalPlanner, PlanContext};

impl PhysicalPlanner {
    /// Whether a matched compound index may serve this query: not an
    /// editorial listing it would reorder, and BUILT for this declaration.
    pub(super) fn compound_candidate_usable(
        &self,
        (index_name, equality_columns, _, claims_order): &CompoundIndexMatch,
        context: &PlanContext,
        workspace: &str,
        branch: &str,
    ) -> bool {
        // A WORKSPACE-owned index pinned to a parent (`__parent_path` among
        // its equality columns) answers a CHILD_OF listing — every child,
        // whatever its type — and, unless it also serves the ORDER BY, would
        // replace the child scan's EDITORIAL order (`ORDER BY __order` /
        // `__tree_order`, which that scan elides, and the order a listing with
        // no ORDER BY has always come back in) with a Sort or with index
        // order: a menu filtered on `status` would silently reorder itself by
        // the index's trailing column. Leave such a listing to the CHILD_OF
        // scan (plan Phase 13e). Untyped CHILD_OF never reached a compound
        // index before workspace indexes, so this keeps its order unchanged.
        let workspace_owned = self.compound_indexes.iter().any(|index| {
            &index.name == index_name
                && matches!(
                    index.owner,
                    Some(
                        raisin_models::nodes::properties::schema::CompoundIndexOwner::Workspace(_)
                    )
                )
        });
        let editorial = match &context.order_by {
            None => true,
            Some((column, _)) => {
                let column = column.to_ascii_lowercase();
                column == "__order" || column == "__tree_order"
            }
        };
        if workspace_owned
            && !claims_order
            && editorial
            && equality_columns
                .iter()
                .any(|(property, _, _)| property == "__parent_path")
        {
            return false;
        }

        // A declaration is not a built index. Consult the persisted build state
        // and DECLINE unless it says Ready for this exact declaration.
        //
        // This must sit between the match and the plan, never earlier: the match
        // is what tells us WHICH index we would use, and availability is
        // per-index. And it must not be skipped for speed — below this point the
        // matched equality predicates are removed from the residual filter, so a
        // scan over an empty or stale keyspace yields missing rows with nothing
        // downstream to catch it.
        let availability = self.compound_availability(workspace, branch, index_name);
        if !availability.is_ready() {
            tracing::debug!(
                index = %index_name,
                workspace = %workspace,
                detail = %availability.explain_reason(),
                "compound index matched the query but is not usable; trying the next candidate"
            );
            return false;
        }
        true
    }
}
