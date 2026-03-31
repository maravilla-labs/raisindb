//! Graph Pattern Matching
//!
//! Matches graph patterns against the storage layer.

mod single_hop;
mod single_node;
mod variable_length;

pub use single_hop::{match_from_source, match_single_hop};
pub use single_node::match_single_node;
pub use variable_length::execute_variable_length_pattern;

use raisin_sql::ast::{NodePattern, PathPattern, PatternElement, RelationshipPattern};

use crate::physical_plan::executor::ExecutionError;

type Result<T> = std::result::Result<T, ExecutionError>;

/// Check if a node type matches a label filter
///
/// Label matching is case-insensitive and supports:
/// - Empty labels list: matches any node type
/// - Single label: exact match (case-insensitive)
/// - Multiple labels: OR match (any label matches)
pub fn matches_label(labels: &[String], node_type: &str) -> bool {
    if labels.is_empty() {
        return true;
    }

    let node_type_lower = node_type.to_lowercase();

    labels.iter().any(|label| {
        let label_lower = label.to_lowercase();
        // Support both exact match and suffix match (e.g., "Article" matches "news:Article")
        node_type_lower == label_lower || node_type_lower.ends_with(&format!(":{}", label_lower))
    })
}

/// Analyze a path pattern to determine its structure
#[derive(Debug)]
pub enum PatternStructure {
    /// Single node: (a)
    SingleNode(NodePattern),
    /// Single hop: (a)-[r]->(b)
    SingleHop {
        source: NodePattern,
        rel: RelationshipPattern,
        target: NodePattern,
    },
    /// Chain of hops: (a)-[r1]->(b)-[r2]->(c)
    Chain(Vec<PatternElement>),
}

/// Analyze a path pattern structure
pub fn analyze_pattern(pattern: &PathPattern) -> Result<PatternStructure> {
    match pattern.elements.as_slice() {
        // Single node
        [PatternElement::Node(node)] => Ok(PatternStructure::SingleNode(node.clone())),

        // Single hop: node-rel-node
        [PatternElement::Node(source), PatternElement::Relationship(rel), PatternElement::Node(target)] => {
            Ok(PatternStructure::SingleHop {
                source: source.clone(),
                rel: rel.clone(),
                target: target.clone(),
            })
        }

        // Chain (3+ nodes)
        elements if elements.len() >= 5 => Ok(PatternStructure::Chain(elements.to_vec())),

        _ => Err(ExecutionError::Validation(
            "Invalid pattern structure".to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_matches_label_empty() {
        assert!(matches_label(&[], "User"));
        assert!(matches_label(&[], "news:Article"));
    }

    #[test]
    fn test_matches_label_exact() {
        assert!(matches_label(&["User".into()], "User"));
        assert!(matches_label(&["user".into()], "User")); // case insensitive
        assert!(!matches_label(&["Admin".into()], "User"));
    }

    #[test]
    fn test_matches_label_with_namespace() {
        assert!(matches_label(&["Article".into()], "news:Article"));
        assert!(matches_label(&["article".into()], "news:Article"));
        assert!(!matches_label(&["User".into()], "news:Article"));
    }

    #[test]
    fn test_matches_label_multiple() {
        let labels = vec!["User".into(), "Admin".into()];
        assert!(matches_label(&labels, "User"));
        assert!(matches_label(&labels, "Admin"));
        assert!(!matches_label(&labels, "Guest"));
    }
}
