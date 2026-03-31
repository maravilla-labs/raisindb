//! Rule matchers and match context for content processing rules.

use serde::{Deserialize, Serialize};

/// Matcher that determines when a rule should apply.
///
/// Multiple matchers can be combined with AND logic using `Combined`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[derive(Default)]
pub enum RuleMatcher {
    /// Matches all nodes (catch-all rule).
    #[default]
    All,

    /// Matches nodes of a specific type (e.g., "raisin:Asset").
    NodeType(String),

    /// Matches nodes at paths matching a glob pattern.
    Path {
        /// Glob pattern (e.g., "/docs/**", "*.pdf", "/images/*").
        pattern: String,
    },

    /// Matches nodes with a specific MIME type.
    MimeType {
        /// The MIME type to match (e.g., "application/pdf", "image/png").
        mime_type: String,
    },

    /// Matches if all contained matchers match (AND logic).
    Combined {
        /// List of matchers that must all match.
        matchers: Vec<RuleMatcher>,
    },

    /// Matches nodes in a specific workspace.
    Workspace {
        /// The workspace name to match.
        workspace: String,
    },

    /// Matches nodes with a property value.
    Property {
        /// Property name to check.
        name: String,
        /// Expected value (string match).
        value: String,
    },
}

impl RuleMatcher {
    /// Check if this matcher matches the given context.
    pub fn matches(&self, context: &RuleMatchContext) -> bool {
        match self {
            RuleMatcher::All => true,

            RuleMatcher::NodeType(node_type) => {
                context.node_type.as_deref() == Some(node_type.as_str())
            }

            RuleMatcher::Path { pattern } => {
                if let Some(ref path) = context.path {
                    glob_match(pattern, path)
                } else {
                    false
                }
            }

            RuleMatcher::MimeType { mime_type } => {
                context.mime_type.as_deref() == Some(mime_type.as_str())
            }

            RuleMatcher::Combined { matchers } => matchers.iter().all(|m| m.matches(context)),

            RuleMatcher::Workspace { workspace } => {
                context.workspace.as_deref() == Some(workspace.as_str())
            }

            RuleMatcher::Property { name, value } => context
                .properties
                .get(name)
                .map(|v| v == value)
                .unwrap_or(false),
        }
    }
}

/// Context information used for rule matching.
///
/// This is passed to rules when evaluating which rule should apply.
#[derive(Debug, Clone, Default)]
pub struct RuleMatchContext {
    /// Node type (e.g., "raisin:Asset").
    pub node_type: Option<String>,

    /// Node path (e.g., "/documents/report.pdf").
    pub path: Option<String>,

    /// MIME type of the content.
    pub mime_type: Option<String>,

    /// Workspace containing the node.
    pub workspace: Option<String>,

    /// Additional properties for matching.
    pub properties: std::collections::HashMap<String, String>,
}

impl RuleMatchContext {
    /// Create a new empty context.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the node type.
    pub fn with_node_type(mut self, node_type: impl Into<String>) -> Self {
        self.node_type = Some(node_type.into());
        self
    }

    /// Set the node path.
    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }

    /// Set the MIME type.
    pub fn with_mime_type(mut self, mime_type: impl Into<String>) -> Self {
        self.mime_type = Some(mime_type.into());
        self
    }

    /// Set the workspace.
    pub fn with_workspace(mut self, workspace: impl Into<String>) -> Self {
        self.workspace = Some(workspace.into());
        self
    }

    /// Add a property for matching.
    pub fn with_property(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.properties.insert(name.into(), value.into());
        self
    }
}

// =============================================================================
// Glob Matching Helper
// =============================================================================

/// Simple glob pattern matching for paths.
///
/// Supports:
/// - `*` - matches any characters within a path segment
/// - `**` - matches any characters including path separators
/// - `?` - matches a single character
pub(crate) fn glob_match(pattern: &str, path: &str) -> bool {
    let pattern_parts: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
    let path_parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    glob_match_parts(&pattern_parts, &path_parts)
}

fn glob_match_parts(pattern: &[&str], path: &[&str]) -> bool {
    if pattern.is_empty() {
        return path.is_empty();
    }

    let first_pattern = pattern[0];

    if first_pattern == "**" {
        if pattern.len() == 1 {
            return true;
        }
        for i in 0..=path.len() {
            if glob_match_parts(&pattern[1..], &path[i..]) {
                return true;
            }
        }
        return false;
    }

    if path.is_empty() {
        return false;
    }

    if segment_matches(first_pattern, path[0]) {
        glob_match_parts(&pattern[1..], &path[1..])
    } else {
        false
    }
}

fn segment_matches(pattern: &str, segment: &str) -> bool {
    if pattern == "*" {
        return true;
    }

    let pattern_chars: Vec<char> = pattern.chars().collect();
    let segment_chars: Vec<char> = segment.chars().collect();

    segment_matches_chars(&pattern_chars, &segment_chars)
}

fn segment_matches_chars(pattern: &[char], segment: &[char]) -> bool {
    if pattern.is_empty() {
        return segment.is_empty();
    }

    match pattern[0] {
        '*' => {
            for i in 0..=segment.len() {
                if segment_matches_chars(&pattern[1..], &segment[i..]) {
                    return true;
                }
            }
            false
        }
        '?' => {
            if segment.is_empty() {
                false
            } else {
                segment_matches_chars(&pattern[1..], &segment[1..])
            }
        }
        c => {
            if segment.is_empty() || segment[0] != c {
                false
            } else {
                segment_matches_chars(&pattern[1..], &segment[1..])
            }
        }
    }
}
