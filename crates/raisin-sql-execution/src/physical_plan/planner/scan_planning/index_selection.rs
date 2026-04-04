//! Index selection heuristics
//!
//! Collects available index options, applies heuristics (e.g., preferring path
//! ordering when ORDER BY path is present), and selects the best predicate to
//! drive the scan.

use super::super::{CanonicalPredicate, PhysicalPlanner};

impl PhysicalPlanner {
    /// Select the best predicate for index-based scanning
    ///
    /// This method applies heuristics to choose between multiple available indexes:
    /// - CHILD_OF, DESCENDANT_OF, and REFERENCES take absolute priority
    /// - When both prefix and property indexes are available, prefer prefix if
    ///   selectivity difference is within 10x (for natural path ordering)
    /// - ORDER BY path strongly favors PrefixScan
    /// - Otherwise, choose the most selective index
    pub(in super::super) fn select_best_predicate<'a>(
        &self,
        index_options: &[(&'a CanonicalPredicate, f64)],
        ordering_by_path: bool,
    ) -> Option<&'a CanonicalPredicate> {
        let has_prefix = index_options
            .iter()
            .any(|(pred, _)| matches!(pred, CanonicalPredicate::PrefixRange { .. }));
        let has_property = index_options.iter().any(|(pred, _)| {
            matches!(
                pred,
                CanonicalPredicate::ColumnEq { .. } | CanonicalPredicate::JsonPropertyEq { .. }
            )
        });
        let has_child_of = index_options
            .iter()
            .any(|(pred, _)| matches!(pred, CanonicalPredicate::ChildOf { .. }));
        let has_descendant_of = index_options
            .iter()
            .any(|(pred, _)| matches!(pred, CanonicalPredicate::DescendantOf { .. }));
        let has_references = index_options
            .iter()
            .any(|(pred, _)| matches!(pred, CanonicalPredicate::References { .. }));

        // CHILD_OF, DESCENDANT_OF, and REFERENCES take priority over other scans
        if has_child_of {
            tracing::debug!(
                "Prioritizing CHILD_OF scan over other indexes (returns naturally ordered children)"
            );
            return index_options
                .iter()
                .find(|(p, _)| matches!(p, CanonicalPredicate::ChildOf { .. }))
                .map(|(p, _)| *p);
        }

        // When SpatialDWithin is present alongside hierarchy scans, let selectivity decide
        // (spatial index is typically more selective than path prefix scans)
        let has_spatial = index_options
            .iter()
            .any(|(pred, _)| matches!(pred, CanonicalPredicate::SpatialDWithin { .. }));

        if has_descendant_of && !has_spatial {
            tracing::debug!(
                "Prioritizing DESCENDANT_OF scan over other indexes (uses efficient path prefix scan)"
            );
            return index_options
                .iter()
                .find(|(p, _)| matches!(p, CanonicalPredicate::DescendantOf { .. }))
                .map(|(p, _)| *p);
        }

        if has_references {
            tracing::debug!(
                "Prioritizing REFERENCES scan (uses reverse reference index for efficient lookup)"
            );
            return index_options
                .iter()
                .find(|(p, _)| matches!(p, CanonicalPredicate::References { .. }))
                .map(|(p, _)| *p);
        }

        if has_prefix && has_property {
            let prefix_sel = index_options
                .iter()
                .find(|(p, _)| matches!(p, CanonicalPredicate::PrefixRange { .. }))
                .map(|(_, s)| *s);
            let property_sel = index_options
                .iter()
                .find(|(p, _)| {
                    matches!(
                        p,
                        CanonicalPredicate::ColumnEq { .. }
                            | CanonicalPredicate::JsonPropertyEq { .. }
                    )
                })
                .map(|(_, s)| *s);

            match (prefix_sel, property_sel, ordering_by_path) {
                (Some(p_sel), Some(prop_sel), true) => {
                    tracing::debug!(
                        "Strongly preferring PrefixScan (sel={:.3}) over PropertyIndexScan (sel={:.3}) due to ORDER BY path",
                        p_sel,
                        prop_sel
                    );
                    return index_options
                        .iter()
                        .find(|(p, _)| matches!(p, CanonicalPredicate::PrefixRange { .. }))
                        .map(|(p, _)| *p);
                }
                (Some(p_sel), Some(prop_sel), false) if p_sel / prop_sel < 10.0 => {
                    tracing::debug!(
                        "Preferring PrefixScan (sel={:.3}) over PropertyIndexScan (sel={:.3}) for natural path ordering",
                        p_sel,
                        prop_sel
                    );
                    return index_options
                        .iter()
                        .find(|(p, _)| matches!(p, CanonicalPredicate::PrefixRange { .. }))
                        .map(|(p, _)| *p);
                }
                _ => {
                    // Default: choose most selective
                    return index_options
                        .iter()
                        .min_by(|a, b| a.1.total_cmp(&b.1))
                        .map(|(p, _)| *p);
                }
            }
        }

        // No conflict - choose most selective
        index_options
            .iter()
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(p, _)| *p)
    }
}
