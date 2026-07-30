//! Edge cost resolution for `ANY CHEAPEST` (a RaisinDB extension).
//!
//! Neither GQL nor SQL/PGQ standardises weighted path search; the committee
//! lists "cheapest path search, by adding weights to edges" among features not
//! ready for the current drafts. The spelling here follows Google Spanner
//! Graph (`ANY CHEAPEST` + `COST <expr>`).
//!
//! # Why a missing weight is an ERROR
//!
//! `build_adjacency_with_weights` historically did `rel.weight.unwrap_or(1.0)`,
//! which is why weighted `sssp()` silently reports hop counts on an unweighted
//! graph. A routing query that answers with a shortest-*hop* path while
//! claiming to be cheapest is the silent-wrong-results class this codebase has
//! spent a whole pass eliminating. So: no defaulting, ever, on this path.

use super::types::{GraphEdge, GraphNodeId};
use thiserror::Error;

/// Why a `COST` expression could not be resolved to a usable edge cost.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum CostError {
    /// A traversed edge carried no weight.
    #[error(
        "edge {relation_type} from {source_node} to {target_node} has no weight; \
         ANY CHEAPEST requires every traversed edge to carry a positive finite weight"
    )]
    MissingWeight {
        /// Relation type of the offending edge.
        relation_type: String,
        /// `workspace:id` of the source node.
        source_node: String,
        /// `workspace:id` of the target node.
        target_node: String,
    },

    /// A traversed edge carried a weight Dijkstra/A* cannot use.
    ///
    /// Non-positive, NaN and infinite weights make the algorithm undefined; it
    /// would return a wrong answer rather than fail, so it fails instead.
    #[error(
        "edge {relation_type} from {source_node} to {target_node} has weight {weight}; \
         ANY CHEAPEST requires every traversed edge to carry a positive finite weight"
    )]
    InvalidWeight {
        /// Relation type of the offending edge.
        relation_type: String,
        /// `workspace:id` of the source node.
        source_node: String,
        /// `workspace:id` of the target node.
        target_node: String,
        /// The rejected weight, rendered.
        weight: String,
    },

    /// `COST <literal>` was not a positive finite number.
    #[error("COST must evaluate to a finite positive number, got {0}")]
    InvalidConstant(String),
}

impl From<CostError> for raisin_error::Error {
    fn from(err: CostError) -> Self {
        raisin_error::Error::Validation(err.to_string())
    }
}

fn render(node: &GraphNodeId) -> String {
    format!("{}:{}", node.0, node.1)
}

/// How to price one edge under `ANY CHEAPEST`.
///
/// `COST` accepts exactly two shapes, because a RaisinDB relation
/// (`RelationRef`) has no property map — `target`, `workspace`,
/// `target_node_type`, `relation_type` and `weight` are all it has, and
/// `weight` is the only numeric one.
#[derive(Debug, Clone, PartialEq)]
pub enum CostSpec {
    /// `COST <edge_variable>.weight` — read `RelationRef::weight` per edge.
    EdgeWeight,
    /// `COST <positive finite numeric literal>`.
    ///
    /// `COST 1` is legal and is equivalent to `ANY SHORTEST`.
    Constant(f64),
}

impl CostSpec {
    /// Build a constant cost, rejecting anything not positive and finite.
    pub fn constant(value: f64) -> Result<Self, CostError> {
        if !value.is_finite() || value <= 0.0 {
            return Err(CostError::InvalidConstant(value.to_string()));
        }
        Ok(Self::Constant(value))
    }

    /// Price one edge leaving `source`.
    pub fn edge_cost(&self, source: &GraphNodeId, edge: &GraphEdge) -> Result<f64, CostError> {
        match self {
            Self::Constant(value) => Ok(*value),
            Self::EdgeWeight => {
                let Some(weight) = edge.weight else {
                    return Err(CostError::MissingWeight {
                        relation_type: edge.relation_type.clone(),
                        source_node: render(source),
                        target_node: render(&edge.target()),
                    });
                };
                let weight = weight as f64;
                if !weight.is_finite() || weight <= 0.0 {
                    return Err(CostError::InvalidWeight {
                        relation_type: edge.relation_type.clone(),
                        source_node: render(source),
                        target_node: render(&edge.target()),
                        weight: weight.to_string(),
                    });
                }
                Ok(weight)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edge(weight: Option<f32>) -> GraphEdge {
        GraphEdge::new("ws", "B", "Account", "Transfers", weight)
    }

    fn source() -> GraphNodeId {
        ("ws".into(), "A".into())
    }

    #[test]
    fn edge_weight_reads_the_relation_weight() {
        assert_eq!(
            CostSpec::EdgeWeight.edge_cost(&source(), &edge(Some(2.5))),
            Ok(2.5)
        );
    }

    #[test]
    fn a_missing_weight_is_an_error_not_a_default_of_one() {
        let err = CostSpec::EdgeWeight
            .edge_cost(&source(), &edge(None))
            .unwrap_err();
        assert!(matches!(err, CostError::MissingWeight { .. }));
        assert!(err.to_string().contains("has no weight"));
    }

    #[test]
    fn non_positive_and_non_finite_weights_are_rejected() {
        for bad in [0.0f32, -1.0, f32::NAN, f32::INFINITY] {
            let err = CostSpec::EdgeWeight
                .edge_cost(&source(), &edge(Some(bad)))
                .unwrap_err();
            assert!(matches!(err, CostError::InvalidWeight { .. }), "{bad}");
        }
    }

    #[test]
    fn constant_cost_rejects_zero_negative_and_non_finite() {
        assert!(CostSpec::constant(1.0).is_ok());
        for bad in [0.0, -3.0, f64::NAN, f64::INFINITY] {
            assert!(CostSpec::constant(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn constant_cost_ignores_the_edge_weight_entirely() {
        let spec = CostSpec::constant(1.0).unwrap();
        assert_eq!(spec.edge_cost(&source(), &edge(None)), Ok(1.0));
    }
}
