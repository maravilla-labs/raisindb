use super::*;
use crate::Dependency;

fn create_manifest(name: &str, version: &str, deps: Vec<(&str, &str)>) -> crate::Manifest {
    crate::Manifest {
        name: name.to_string(),
        version: version.to_string(),
        dependencies: deps
            .into_iter()
            .map(|(n, v)| Dependency {
                name: n.to_string(),
                version: v.to_string(),
            })
            .collect(),
        ..Default::default()
    }
}

#[test]
fn test_simple_dependency_order() {
    let mut graph = DependencyGraph::new();

    // A depends on B, B depends on C
    graph.add_package(create_manifest("C", "1.0.0", vec![]));
    graph.add_package(create_manifest("B", "1.0.0", vec![("C", "1.0.0")]));
    graph.add_package(create_manifest("A", "1.0.0", vec![("B", "1.0.0")]));

    let order = graph.installation_order().unwrap();

    // C should come before B, B should come before A
    let c_pos = order.iter().position(|n| n == "C").unwrap();
    let b_pos = order.iter().position(|n| n == "B").unwrap();
    let a_pos = order.iter().position(|n| n == "A").unwrap();

    assert!(c_pos < b_pos);
    assert!(b_pos < a_pos);
}

#[test]
fn test_no_dependencies() {
    let mut graph = DependencyGraph::new();

    graph.add_package(create_manifest("A", "1.0.0", vec![]));
    graph.add_package(create_manifest("B", "1.0.0", vec![]));
    graph.add_package(create_manifest("C", "1.0.0", vec![]));

    let order = graph.installation_order().unwrap();
    assert_eq!(order.len(), 3);
}

#[test]
fn test_circular_dependency() {
    let mut graph = DependencyGraph::new();

    // A -> B -> C -> A (cycle)
    graph.add_package(create_manifest("A", "1.0.0", vec![("B", "1.0.0")]));
    graph.add_package(create_manifest("B", "1.0.0", vec![("C", "1.0.0")]));
    graph.add_package(create_manifest("C", "1.0.0", vec![("A", "1.0.0")]));

    let result = graph.installation_order();
    assert!(result.is_err());

    match result.unwrap_err() {
        DependencyGraphError::CircularDependency { cycle } => {
            assert!(cycle.len() >= 3);
            // Cycle should include A, B, C
            assert!(cycle.contains(&"A".to_string()));
            assert!(cycle.contains(&"B".to_string()));
            assert!(cycle.contains(&"C".to_string()));
        }
        _ => panic!("Expected CircularDependency error"),
    }
}

#[test]
fn test_diamond_dependency() {
    let mut graph = DependencyGraph::new();

    // Diamond: A depends on B and C, both B and C depend on D
    graph.add_package(create_manifest("D", "1.0.0", vec![]));
    graph.add_package(create_manifest("B", "1.0.0", vec![("D", "1.0.0")]));
    graph.add_package(create_manifest("C", "1.0.0", vec![("D", "1.0.0")]));
    graph.add_package(create_manifest(
        "A",
        "1.0.0",
        vec![("B", "1.0.0"), ("C", "1.0.0")],
    ));

    let order = graph.installation_order().unwrap();

    // D should come first, then B and C (in any order), then A
    let d_pos = order.iter().position(|n| n == "D").unwrap();
    let b_pos = order.iter().position(|n| n == "B").unwrap();
    let c_pos = order.iter().position(|n| n == "C").unwrap();
    let a_pos = order.iter().position(|n| n == "A").unwrap();

    assert!(d_pos < b_pos);
    assert!(d_pos < c_pos);
    assert!(b_pos < a_pos);
    assert!(c_pos < a_pos);
}

#[test]
fn test_missing_dependency() {
    let mut graph = DependencyGraph::new();

    // A depends on B, but B is not in the graph
    graph.add_package(create_manifest("A", "1.0.0", vec![("B", "1.0.0")]));

    let result = graph.validate_dependencies();
    assert!(result.is_err());

    match result.unwrap_err() {
        DependencyGraphError::MissingDependency {
            package,
            dependency,
        } => {
            assert_eq!(package, "A");
            assert_eq!(dependency, "B");
        }
        _ => panic!("Expected MissingDependency error"),
    }
}
