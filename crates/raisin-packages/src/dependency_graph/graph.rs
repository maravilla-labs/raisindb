// SPDX-License-Identifier: BSL-1.1

//! Dependency graph core: graph building, topological sort, and cycle detection.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::Manifest;

/// A node in the dependency graph
#[derive(Debug, Clone)]
pub struct PackageNode {
    /// Package name
    pub name: String,
    /// Package version
    pub version: String,
    /// Direct dependencies (package names)
    pub dependencies: Vec<String>,
    /// Package manifest
    pub manifest: Manifest,
}

/// Error types for dependency graph operations
#[derive(Debug, Clone)]
pub enum DependencyGraphError {
    /// Circular dependency detected
    CircularDependency {
        /// The cycle path (list of package names)
        cycle: Vec<String>,
    },
    /// Missing dependency
    MissingDependency {
        /// Package that has the missing dependency
        package: String,
        /// The missing dependency name
        dependency: String,
    },
    /// Type reference not found
    TypeReferenceNotFound {
        /// Package with the type reference
        package: String,
        /// Type of reference (nodetype, archetype, elementtype)
        reference_type: String,
        /// Name of the missing type
        name: String,
    },
}

impl std::fmt::Display for DependencyGraphError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DependencyGraphError::CircularDependency { cycle } => {
                writeln!(f, "Circular dependency detected in package dependencies:")?;
                for (i, pkg) in cycle.iter().enumerate() {
                    if i == 0 {
                        writeln!(f, "  > {}", pkg)?;
                    } else if i == cycle.len() - 1 {
                        writeln!(f, "    |")?;
                        writeln!(f, "    v")?;
                        writeln!(f, "  > {} (cycle)", pkg)?;
                    } else {
                        writeln!(f, "    |")?;
                        writeln!(f, "    v")?;
                        writeln!(f, "    {}", pkg)?;
                    }
                }
                write!(
                    f,
                    "\nTo resolve: Remove one of these dependency relationships."
                )
            }
            DependencyGraphError::MissingDependency {
                package,
                dependency,
            } => {
                write!(
                    f,
                    "Missing dependency: package '{}' requires '{}' which is not available",
                    package, dependency
                )
            }
            DependencyGraphError::TypeReferenceNotFound {
                package,
                reference_type,
                name,
            } => {
                write!(
                    f,
                    "Type reference not found: {} '{}' referenced in package '{}' does not exist",
                    reference_type, name, package
                )
            }
        }
    }
}

impl std::error::Error for DependencyGraphError {}

/// Dependency graph for package installation
#[derive(Debug, Clone, Default)]
pub struct DependencyGraph {
    /// Package nodes by name
    nodes: HashMap<String, PackageNode>,
    /// Adjacency list (package -> list of packages that depend on it)
    reverse_adjacency: HashMap<String, Vec<String>>,
}

impl DependencyGraph {
    /// Create a new empty dependency graph
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a package to the graph
    pub fn add_package(&mut self, manifest: Manifest) {
        let dependencies: Vec<String> = manifest
            .dependencies
            .iter()
            .map(|d| d.name.clone())
            .collect();

        // Update reverse adjacency (who depends on what)
        for dep in &dependencies {
            self.reverse_adjacency
                .entry(dep.clone())
                .or_default()
                .push(manifest.name.clone());
        }

        let node = PackageNode {
            name: manifest.name.clone(),
            version: manifest.version.clone(),
            dependencies,
            manifest,
        };

        self.nodes.insert(node.name.clone(), node);
    }

    /// Check if a package is in the graph
    pub fn contains(&self, name: &str) -> bool {
        self.nodes.contains_key(name)
    }

    /// Get a package node by name
    pub fn get(&self, name: &str) -> Option<&PackageNode> {
        self.nodes.get(name)
    }

    /// Get all package names
    pub fn package_names(&self) -> Vec<&str> {
        self.nodes.keys().map(|s| s.as_str()).collect()
    }

    /// Validate that all dependencies exist
    pub fn validate_dependencies(&self) -> Result<(), DependencyGraphError> {
        for (pkg_name, node) in &self.nodes {
            for dep in &node.dependencies {
                if !self.nodes.contains_key(dep) {
                    return Err(DependencyGraphError::MissingDependency {
                        package: pkg_name.clone(),
                        dependency: dep.clone(),
                    });
                }
            }
        }
        Ok(())
    }

    /// Perform topological sort using Kahn's algorithm
    /// Returns packages in installation order (dependencies first)
    pub fn topological_sort(&self) -> Result<Vec<String>, DependencyGraphError> {
        // Calculate in-degree for each node (number of dependencies)
        let mut in_degree: HashMap<String, usize> = HashMap::new();
        for (name, node) in &self.nodes {
            in_degree.entry(name.clone()).or_insert(0);
            for dep in &node.dependencies {
                // dep has one more dependent (this package)
                in_degree.entry(dep.clone()).or_insert(0);
            }
        }

        // For each package, count its dependencies that are in our graph
        for (name, node) in &self.nodes {
            let dep_count = node
                .dependencies
                .iter()
                .filter(|d| self.nodes.contains_key(*d))
                .count();
            *in_degree.get_mut(name).unwrap() = dep_count;
        }

        // Start with nodes that have no dependencies (in-degree = 0)
        let mut queue: VecDeque<String> = in_degree
            .iter()
            .filter(|(name, &degree)| degree == 0 && self.nodes.contains_key(*name))
            .map(|(name, _)| name.clone())
            .collect();

        let mut result = Vec::new();

        while let Some(current) = queue.pop_front() {
            result.push(current.clone());

            // For each package that depends on current
            if let Some(dependents) = self.reverse_adjacency.get(&current) {
                for dependent in dependents {
                    if let Some(degree) = in_degree.get_mut(dependent) {
                        *degree -= 1;
                        if *degree == 0 {
                            queue.push_back(dependent.clone());
                        }
                    }
                }
            }
        }

        // If we couldn't process all nodes, there's a cycle
        if result.len() != self.nodes.len() {
            // Find the cycle
            let cycle = self.find_cycle();
            return Err(DependencyGraphError::CircularDependency { cycle });
        }

        Ok(result)
    }

    /// Find a cycle in the graph using DFS
    fn find_cycle(&self) -> Vec<String> {
        let mut visited = HashSet::new();
        let mut rec_stack = HashSet::new();
        let mut path = Vec::new();

        for name in self.nodes.keys() {
            if !visited.contains(name) {
                if let Some(cycle) =
                    self.find_cycle_dfs(name, &mut visited, &mut rec_stack, &mut path)
                {
                    return cycle;
                }
            }
        }

        Vec::new() // Should not happen if topological_sort detected a cycle
    }

    /// DFS helper for cycle detection
    fn find_cycle_dfs(
        &self,
        node: &str,
        visited: &mut HashSet<String>,
        rec_stack: &mut HashSet<String>,
        path: &mut Vec<String>,
    ) -> Option<Vec<String>> {
        visited.insert(node.to_string());
        rec_stack.insert(node.to_string());
        path.push(node.to_string());

        if let Some(pkg_node) = self.nodes.get(node) {
            for dep in &pkg_node.dependencies {
                if !visited.contains(dep) {
                    if let Some(cycle) = self.find_cycle_dfs(dep, visited, rec_stack, path) {
                        return Some(cycle);
                    }
                } else if rec_stack.contains(dep) {
                    // Found cycle: extract path from dep to current node
                    let cycle_start = path.iter().position(|n| n == dep).unwrap();
                    let mut cycle: Vec<String> = path[cycle_start..].to_vec();
                    cycle.push(dep.clone()); // Complete the cycle
                    return Some(cycle);
                }
            }
        }

        rec_stack.remove(node);
        path.pop();
        None
    }

    /// Get packages in installation order (wrapper for topological_sort)
    pub fn installation_order(&self) -> Result<Vec<String>, DependencyGraphError> {
        self.topological_sort()
    }

    /// Get the manifests in installation order
    pub fn manifests_in_order(&self) -> Result<Vec<&Manifest>, DependencyGraphError> {
        let order = self.installation_order()?;
        Ok(order
            .iter()
            .filter_map(|name| self.nodes.get(name).map(|n| &n.manifest))
            .collect())
    }
}
