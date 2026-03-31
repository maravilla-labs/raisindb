//! Scope filtering for graph algorithm computations
//!
//! Filters nodes based on path patterns, node types, workspaces, and relation types.

use super::types::GraphScope;
use glob::Pattern;

/// A compiled scope filter for efficient node matching
#[derive(Debug)]
pub struct ScopeFilter {
    /// Compiled path patterns (glob syntax)
    path_patterns: Vec<Pattern>,
    /// Node types to include
    node_types: Vec<String>,
    /// Workspaces to include
    workspaces: Vec<String>,
    /// Relation types to filter by
    relation_types: Vec<String>,
    /// Whether the scope is empty (matches all)
    is_empty: bool,
}

impl ScopeFilter {
    /// Create a new scope filter from a GraphScope configuration
    ///
    /// Alias for `new` for more descriptive usage
    pub fn from_scope(scope: &GraphScope) -> Self {
        Self::new(scope)
    }

    /// Create a new scope filter from a GraphScope configuration
    pub fn new(scope: &GraphScope) -> Self {
        let path_patterns: Vec<Pattern> = scope
            .paths
            .iter()
            .filter_map(|p| Pattern::new(p).ok())
            .collect();

        let is_empty = scope.paths.is_empty()
            && scope.node_types.is_empty()
            && scope.workspaces.is_empty()
            && scope.relation_types.is_empty();

        Self {
            path_patterns,
            node_types: scope.node_types.clone(),
            workspaces: scope.workspaces.clone(),
            relation_types: scope.relation_types.clone(),
            is_empty,
        }
    }

    /// Check if a node matches the scope filter
    ///
    /// Returns true if:
    /// - The scope is empty (matches all nodes)
    /// - OR the node matches at least one of the configured criteria
    pub fn matches(&self, path: &str, node_type: &str, workspace: &str) -> bool {
        // Empty scope means "match all"
        if self.is_empty {
            return true;
        }

        // Check path patterns
        if !self.path_patterns.is_empty() {
            let matches_path = self.path_patterns.iter().any(|p| p.matches(path));
            if matches_path {
                return true;
            }
        }

        // Check node types
        if !self.node_types.is_empty() && self.node_types.contains(&node_type.to_string()) {
            return true;
        }

        // Check workspaces
        if !self.workspaces.is_empty() && self.workspaces.contains(&workspace.to_string()) {
            return true;
        }

        false
    }

    /// Check if a relation type is in scope
    ///
    /// Returns true if:
    /// - No relation_types filter is configured (all relations included)
    /// - OR the relation type is in the configured list
    pub fn matches_relation_type(&self, rel_type: &str) -> bool {
        if self.relation_types.is_empty() {
            return true;
        }
        self.relation_types.contains(&rel_type.to_string())
    }

    /// Get the list of workspaces in scope
    pub fn workspaces(&self) -> &[String] {
        &self.workspaces
    }

    /// Get the list of node types in scope
    pub fn node_types(&self) -> &[String] {
        &self.node_types
    }

    /// Get the list of relation types in scope
    pub fn relation_types(&self) -> &[String] {
        &self.relation_types
    }

    /// Check if scope has path filters
    pub fn has_path_filters(&self) -> bool {
        !self.path_patterns.is_empty()
    }

    /// Check if scope has node type filters
    pub fn has_node_type_filters(&self) -> bool {
        !self.node_types.is_empty()
    }

    /// Check if scope has workspace filters
    pub fn has_workspace_filters(&self) -> bool {
        !self.workspaces.is_empty()
    }

    /// Check if scope has relation type filters
    pub fn has_relation_type_filters(&self) -> bool {
        !self.relation_types.is_empty()
    }

    /// Check if the scope is empty (matches all)
    pub fn is_empty(&self) -> bool {
        self.is_empty
    }
}

impl Default for ScopeFilter {
    fn default() -> Self {
        Self {
            path_patterns: Vec::new(),
            node_types: Vec::new(),
            workspaces: Vec::new(),
            relation_types: Vec::new(),
            is_empty: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_scope_matches_all() {
        let scope = GraphScope::default();
        let filter = ScopeFilter::new(&scope);

        assert!(filter.is_empty());
        assert!(filter.matches("any/path", "any:Type", "any_workspace"));
        assert!(filter.matches_relation_type("ANY_RELATION"));
    }

    #[test]
    fn test_path_pattern_matching() {
        let scope = GraphScope {
            paths: vec!["social/users/**".to_string(), "community/**".to_string()],
            ..Default::default()
        };
        let filter = ScopeFilter::new(&scope);

        assert!(filter.matches("social/users/alice", "raisin:User", "social"));
        assert!(filter.matches("social/users/bob/profile", "raisin:Profile", "social"));
        assert!(filter.matches("community/group1", "raisin:Group", "community"));
        assert!(!filter.matches("private/data", "raisin:Data", "private"));
    }

    #[test]
    fn test_node_type_matching() {
        let scope = GraphScope {
            node_types: vec!["raisin:User".to_string(), "raisin:Profile".to_string()],
            ..Default::default()
        };
        let filter = ScopeFilter::new(&scope);

        assert!(filter.matches("any/path", "raisin:User", "any"));
        assert!(filter.matches("any/path", "raisin:Profile", "any"));
        assert!(!filter.matches("any/path", "raisin:Post", "any"));
    }

    #[test]
    fn test_workspace_matching() {
        let scope = GraphScope {
            workspaces: vec!["social".to_string(), "community".to_string()],
            ..Default::default()
        };
        let filter = ScopeFilter::new(&scope);

        assert!(filter.matches("any/path", "any:Type", "social"));
        assert!(filter.matches("any/path", "any:Type", "community"));
        assert!(!filter.matches("any/path", "any:Type", "private"));
    }

    #[test]
    fn test_relation_type_filtering() {
        let scope = GraphScope {
            relation_types: vec!["FRIENDS_WITH".to_string(), "FOLLOWS".to_string()],
            ..Default::default()
        };
        let filter = ScopeFilter::new(&scope);

        assert!(filter.matches_relation_type("FRIENDS_WITH"));
        assert!(filter.matches_relation_type("FOLLOWS"));
        assert!(!filter.matches_relation_type("BLOCKS"));
    }

    #[test]
    fn test_combined_scope() {
        let scope = GraphScope {
            paths: vec!["social/**".to_string()],
            node_types: vec!["raisin:Post".to_string()],
            workspaces: vec!["blog".to_string()],
            relation_types: vec!["AUTHORED".to_string()],
        };
        let filter = ScopeFilter::new(&scope);

        // Matches via path
        assert!(filter.matches("social/posts", "raisin:User", "social"));

        // Matches via node type
        assert!(filter.matches("blog/articles", "raisin:Post", "blog"));

        // Matches via workspace
        assert!(filter.matches("blog/articles", "raisin:Article", "blog"));

        // Doesn't match anything
        assert!(!filter.matches("private/data", "raisin:Data", "private"));
    }
}
