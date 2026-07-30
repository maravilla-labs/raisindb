//! Graph pattern types for SQL/PGQ
//!
//! Defines node patterns, relationship patterns, path patterns, direction,
//! and path quantifiers used in MATCH clauses.

use serde::{Deserialize, Serialize};

use super::expressions::Expr;
use super::query::SourceSpan;
use super::selectors::{PathRestrictor, PathSelector, QuantifierSyntax};

/// A single path pattern: nodes connected by relationships
///
/// ```sql
/// (a:User)-[:follows]->(b:User)-[:likes]->(c:Post)
/// p = (a:User)-[:follows]->{1,3}(b:User)
/// ALL SHORTEST TRAIL p = (a:User)-[:follows]->{1,3}(b:User)
/// ```
///
/// The optional [`variable`](Self::variable) binds the whole path so the path
/// accessors (`path_length(p)`, `nodes(p)`, `edges(p)`, …) can address it. A
/// path variable is not selectable on its own — there is no `PATH` column type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PathPattern {
    /// Optional path variable (`p = ...`)
    #[serde(default)]
    pub variable: Option<String>,
    /// Optional path selector (`ANY SHORTEST`, `ALL SHORTEST`, …)
    #[serde(default)]
    pub selector: Option<PathSelector>,
    /// Optional path restrictor (`WALK`, `TRAIL`, `ACYCLIC`)
    #[serde(default)]
    pub restrictor: Option<PathRestrictor>,
    /// Alternating sequence of nodes and relationships
    pub elements: Vec<PatternElement>,
    /// Source location
    #[serde(default)]
    pub span: SourceSpan,
}

impl PathPattern {
    /// Build a bare path pattern with no variable, selector or restrictor.
    pub fn new(elements: Vec<PatternElement>) -> Self {
        Self {
            variable: None,
            selector: None,
            restrictor: None,
            elements,
            span: SourceSpan::empty(),
        }
    }

    /// The restrictor in force, defaulting to [`PathRestrictor::DEFAULT`].
    pub fn effective_restrictor(&self) -> PathRestrictor {
        self.restrictor.unwrap_or(PathRestrictor::DEFAULT)
    }

    /// Iterate the relationship patterns of this path in order.
    pub fn relationships(&self) -> impl Iterator<Item = &RelationshipPattern> {
        self.elements.iter().filter_map(|e| match e {
            PatternElement::Relationship(r) => Some(r),
            PatternElement::Node(_) => None,
        })
    }
}

/// Element in a path pattern
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PatternElement {
    /// Node pattern
    Node(NodePattern),
    /// Relationship pattern
    Relationship(RelationshipPattern),
}

/// Node pattern
///
/// ```sql
/// (n)                            -- any node
/// (n:User)                       -- with label
/// (n:User|Admin)                 -- multiple labels (OR)
/// ```
///
/// Inline `WHERE` inside a node pattern is **rejected at parse time**: it used
/// to parse into a field nothing ever read, which silently returned unfiltered
/// rows. Predicates belong in the `GRAPH_TABLE` `WHERE` clause.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodePattern {
    /// Optional variable binding
    pub variable: Option<String>,
    /// Labels (maps to node_type)
    pub labels: Vec<String>,
    /// Source location
    #[serde(default)]
    pub span: SourceSpan,
}

impl NodePattern {
    /// Create anonymous node pattern
    pub fn anonymous() -> Self {
        Self {
            variable: None,
            labels: vec![],
            span: SourceSpan::empty(),
        }
    }

    /// Create node with variable
    pub fn with_var(var: impl Into<String>) -> Self {
        Self {
            variable: Some(var.into()),
            labels: vec![],
            span: SourceSpan::empty(),
        }
    }
}

/// Relationship pattern
///
/// ```sql
/// -[r]->                    -- any type, right direction
/// -[:follows]->             -- specific type
/// -[:follows|likes]->       -- multiple types (OR)
/// -[r:follows]->{2}         -- exactly 2 hops
/// -[r:follows]->{1,3}       -- 1 to 3 hops
/// -[t:Transfers COST t.weight]->{1,3}   -- weighted, requires ANY CHEAPEST
/// <-[r]-                    -- left direction
/// -[r]-                     -- any direction
/// -[r:follows*1..3]->       -- deprecated Cypher-style quantifier
/// ```
///
/// Inline `WHERE` inside a relationship pattern is **rejected at parse time**,
/// for the same reason as on [`NodePattern`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RelationshipPattern {
    /// Optional variable binding
    pub variable: Option<String>,
    /// Relationship types (maps to relation_type); empty means any type.
    ///
    /// All entries matter: `-[:a|b]->` matches either type.
    pub types: Vec<String>,
    /// Direction
    pub direction: Direction,
    /// Path quantifier for variable-length paths
    pub quantifier: Option<PathQuantifier>,
    /// `COST <edge>.weight` — RaisinDB extension, only under `ANY CHEAPEST`
    #[serde(default)]
    pub cost: Option<Box<Expr>>,
    /// Source location
    #[serde(default)]
    pub span: SourceSpan,
}

/// Relationship direction
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    /// (a)-[r]->(b) : a to b
    Right,
    /// (a)<-[r]-(b) : b to a
    Left,
    /// (a)-[r]-(b) : either direction
    Any,
}

/// Path quantifier for variable-length paths
///
/// The canonical form is the standard postfix brace form, written **after** the
/// arrow:
///
/// ```sql
/// ->{2}     -- exactly 2
/// ->{1,3}   -- 1 to 3 inclusive
/// ->{2,}    -- 2 or more (unbounded)
/// ->*       -- {0,}
/// ->+       -- {1,}
/// ->?       -- {0,1}
/// ```
///
/// The Cypher-style form written **inside** the brackets (`-[:t*1..3]->`) is a
/// deprecated compatibility alias; see [`QuantifierSyntax`]. Note the two
/// spellings of `*` do not agree: legacy `*` is `{1,}`, standard `*` is `{0,}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathQuantifier {
    /// Minimum hops
    pub min: u32,
    /// Maximum hops (None = unbounded, capped at [`Self::DEFAULT_MAX`])
    pub max: Option<u32>,
    /// Which spelling this quantifier was written in
    #[serde(default)]
    pub syntax: QuantifierSyntax,
}

impl PathQuantifier {
    /// Default maximum path length
    pub const DEFAULT_MAX: u32 = 10;

    /// Legacy unbounded: `*` inside the brackets, meaning `{1,}`
    pub fn unbounded() -> Self {
        Self {
            min: 1,
            max: None,
            syntax: QuantifierSyntax::LegacyStar,
        }
    }

    /// Exact hop count, standard form: `->{n}`
    pub fn exact(n: u32) -> Self {
        Self {
            min: n,
            max: Some(n),
            syntax: QuantifierSyntax::Standard,
        }
    }

    /// Inclusive range, standard form: `->{min,max}`
    pub fn range(min: u32, max: u32) -> Self {
        Self {
            min,
            max: Some(max),
            syntax: QuantifierSyntax::Standard,
        }
    }

    /// Open-ended range, standard form: `->{min,}`
    pub fn at_least(min: u32) -> Self {
        Self {
            min,
            max: None,
            syntax: QuantifierSyntax::Standard,
        }
    }

    /// Get effective maximum
    pub fn effective_max(&self) -> u32 {
        self.max.unwrap_or(Self::DEFAULT_MAX)
    }

    /// True when no upper bound was written.
    ///
    /// Under [`QuantifierSyntax::Standard`] this requires a path selector or a
    /// path restrictor on the containing path; the legacy form predates that
    /// rule and is capped at [`Self::DEFAULT_MAX`] instead.
    pub fn is_unbounded(&self) -> bool {
        self.max.is_none()
    }

    /// True when written in the deprecated Cypher-style form.
    pub fn is_legacy(&self) -> bool {
        matches!(self.syntax, QuantifierSyntax::LegacyStar)
    }

    /// Deprecation text for the planner warning and EXPLAIN output.
    ///
    /// `None` for the canonical form. Callers must surface this — accepting two
    /// dialect spellings *silently* is the thing the deprecation exists to
    /// prevent.
    pub fn deprecation_note(&self) -> Option<String> {
        if !self.is_legacy() {
            return None;
        }
        let written = match self.max {
            Some(max) if max == self.min => format!("*{}", self.min),
            Some(max) => format!("*{}..{}", self.min, max),
            None => format!("*{}..", self.min),
        };
        let canonical = match self.max {
            Some(max) if max == self.min => format!("->{{{}}}", self.min),
            Some(max) => format!("->{{{},{}}}", self.min, max),
            None => format!("->{{{},}}", self.min),
        };
        Some(format!(
            "quantifier {written} (deprecated Cypher-style form; write {canonical})"
        ))
    }
}
