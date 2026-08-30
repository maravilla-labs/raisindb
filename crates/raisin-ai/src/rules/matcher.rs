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

    /// Matches nodes by MIME type, exactly or by family.
    ///
    /// This is the routing table's primary key: an asset pipeline is "route by
    /// mimetype to a set of tasks", and the pattern here is the mimetype half.
    /// See [`mime_matches`] for the accepted forms.
    MimeType {
        /// The MIME pattern to match: `application/pdf` (exact),
        /// `image/*` (family), `image/` (legacy prefix), or `*` / `*/*` (any).
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

            RuleMatcher::MimeType { mime_type } => context
                .mime_type
                .as_deref()
                .map(|actual| mime_matches(mime_type, actual))
                .unwrap_or(false),

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
// MIME Matching Helper
// =============================================================================

/// Match a MIME pattern from a processing rule against a node's actual MIME type.
///
/// # Why this is not `==`
///
/// It used to be. That made [`ProcessingRuleSet::default_rules`] ship an image
/// rule matching the literal string `"image/"` — a value no asset can ever
/// carry, because an upload's mime type is `image/png` or `image/jpeg`. The
/// shipped default for every image in every installation therefore matched
/// nothing, silently fell through to the catch-all rule, and
/// `ProcessingSettings::image()` was unreachable code. There was also no way to
/// write the rule correctly: exact equality cannot express a family, so an
/// operator wanting "all images" had to enumerate every subtype they could
/// think of and quietly miss `image/avif`.
///
/// A media pipeline routes by FAMILY far more often than by exact type — "every
/// image gets a thumbnail", "every video gets a poster frame" — so the family
/// form is the one that has to work.
///
/// # Accepted forms
///
/// * `application/pdf` — exact, case-insensitive.
/// * `image/*` — family: any subtype of `image`.
/// * `image/` — the same family, in the trailing-slash spelling the shipped
///   defaults already used. Kept so existing persisted rule sets start working
///   rather than needing a migration.
/// * `*` or `*/*` — any mime type at all. (`RuleMatcher::All` matches a node
///   with NO mime type too; this one still requires one to be present, which is
///   how you say "any asset with bytes".)
///
/// Parameters after a `;` are ignored on the actual value, so
/// `text/plain; charset=utf-8` matches the pattern `text/plain`. A pattern is
/// never parameterised — an operator writing one would expect it to match, and
/// silently never matching is the failure this function exists to end.
///
/// # Not a glob
///
/// Deliberately: `image/pn*` is not supported and matches nothing. MIME types
/// are a two-level tree, not a path, and a partial-subtype pattern has no
/// meaning anyone would agree on. Widening this to [`glob_match`] would also
/// make `*` match across the `/`, so `image/*` would start matching
/// `application/pdf`.
pub fn mime_matches(pattern: &str, actual: &str) -> bool {
    let pattern = pattern.trim();
    // Strip parameters from the actual value only: `text/plain; charset=utf-8`.
    let actual = actual.split(';').next().unwrap_or(actual).trim();

    if pattern.is_empty() || actual.is_empty() {
        return false;
    }

    if pattern == "*" || pattern == "*/*" {
        return true;
    }

    // Family form: `image/*` or the legacy `image/`.
    if let Some(family) = pattern
        .strip_suffix("/*")
        .or_else(|| pattern.strip_suffix('/'))
    {
        return actual
            .split('/')
            .next()
            .is_some_and(|t| t.eq_ignore_ascii_case(family));
    }

    pattern.eq_ignore_ascii_case(actual)
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
