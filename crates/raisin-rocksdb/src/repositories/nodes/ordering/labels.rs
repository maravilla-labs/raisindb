//! Minting `ORDERED_CHILDREN` order labels.
//!
//! A full order label is `{fractional}::{HLC_hex:016x}`. The fractional part
//! carries the editorial order; the HLC suffix breaks ties deterministically
//! across a cluster when two nodes independently mint the same fractional part
//! (which a branch merge can also produce).
//!
//! Every append site funnels through [`NodeRepositoryImpl::next_append_label`].
//! Before this existed, the four mint sites drifted apart in two ways that both
//! caused real corruption:
//!
//!  1. The non-transactional create/update paths passed the **full** previous
//!     label to `fractional_index::inc`, which parses hex and therefore chokes
//!     on the `::` separator. It failed, fell back to `first()`, and minted a
//!     **duplicate** label — so appending under a parent whose last label came
//!     from the transaction path silently collided.
//!  2. Those same paths omitted the HLC suffix entirely, so the two create paths
//!     produced structurally different labels for the same operation.

use super::super::NodeRepositoryImpl;
use crate::fractional_index;
use raisin_error::Result;
use raisin_hlc::HLC;

impl NodeRepositoryImpl {
    /// Mint the label for appending a new last child under `parent_id`.
    ///
    /// Reads the parent's current last label (O(1) via the `LAST` metadata
    /// cache), strips its HLC suffix, increments the fractional part, and
    /// re-attaches `revision` as the new suffix.
    ///
    /// A corrupt or unparsable previous label is not fatal: it logs and restarts
    /// from `first()`, matching the pre-existing tolerance of these paths.
    pub(in crate::repositories::nodes) fn next_append_label(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent_id: &str,
        revision: &HLC,
    ) -> Result<String> {
        let last_label =
            self.get_last_order_label(tenant_id, repo_id, branch, workspace, parent_id)?;

        let fractional = match last_label.as_deref() {
            Some(last) => {
                // Strip the `::{HLC}` suffix first — `inc` parses hex and would
                // reject the separator.
                let previous = fractional_index::extract_fractional(last);
                match fractional_index::inc(previous) {
                    Ok(label) => label,
                    Err(e) => {
                        tracing::warn!(
                            parent_id = %parent_id,
                            last_label = %last,
                            error = %e,
                            "Corrupt order label detected, falling back to first()"
                        );
                        fractional_index::first()
                    }
                }
            }
            None => fractional_index::first(),
        };

        Ok(format_order_label(&fractional, revision))
    }
}

/// Assemble a full order label from its fractional part and revision.
///
/// Re-exported at this path for the ordering module's callers; the format itself
/// is defined once in [`fractional_index::format_label`].
pub(in crate::repositories::nodes) use fractional_index::format_label as format_order_label;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_round_trips_through_extract_fractional() {
        let revision = HLC::new(1_700_000_000_000, 42);
        let fractional = fractional_index::first();

        let label = format_order_label(&fractional, &revision);
        assert_eq!(fractional_index::extract_fractional(&label), fractional);
        assert!(label.contains(fractional_index::SEPARATOR));
    }

    /// The bug this module exists to prevent: incrementing a full label (rather
    /// than its fractional part) fails, and a caller that falls back to
    /// `first()` mints a duplicate.
    #[test]
    fn inc_rejects_a_full_label_but_accepts_its_fractional_part() {
        let revision = HLC::new(1, 1);
        let first = fractional_index::first();
        let full = format_order_label(&first, &revision);

        assert!(
            fractional_index::inc(&full).is_err(),
            "inc must reject a full label — this is why extract_fractional comes first"
        );

        let next = fractional_index::inc(fractional_index::extract_fractional(&full))
            .expect("inc must accept the fractional part");
        assert!(
            next > first,
            "incremented label must sort after the previous"
        );
    }
}
