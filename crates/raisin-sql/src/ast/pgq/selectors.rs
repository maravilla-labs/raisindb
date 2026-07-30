//! Path selectors, path restrictors and quantifier syntax flavours for SQL/PGQ.
//!
//! A top-level path pattern may be prefixed by a *selector* (which of the
//! matching paths to keep) and a *restrictor* (which walks count as a path at
//! all). The order is fixed: selector first, restrictor second.
//!
//! ```sql
//! MATCH ALL SHORTEST TRAIL p = (a:Account)-[t:Transfers]->{1,3}(b:Account)
//! ```
//!
//! Neither GQL nor SQL/PGQ standardises weighted path search, so
//! [`PathSelector::AnyCheapest`] is a RaisinDB extension whose spelling follows
//! Google Spanner Graph. Portable queries should use
//! [`PathSelector::AnyShortest`], which is hop count only.

use serde::{Deserialize, Serialize};

/// Which of the paths matching a pattern are kept.
///
/// Placed at the head of a top-level path pattern, before any restrictor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PathSelector {
    /// `ANY` — one arbitrary path per endpoint pair.
    ///
    /// Note this is **not** `ANY SHORTEST`: no minimality is promised, only
    /// that the path satisfies the pattern and its quantifiers.
    Any,
    /// `ANY SHORTEST` — one minimum-hop path per endpoint pair.
    AnyShortest,
    /// `ALL SHORTEST` — every minimum-hop path per endpoint pair.
    AllShortest,
    /// `ANY CHEAPEST` — one minimum-cost path per endpoint pair.
    ///
    /// RaisinDB extension. Requires a `COST` clause on at least one edge of the
    /// same path; see [`crate::ast::pgq::RelationshipPattern::cost`].
    AnyCheapest,
}

impl PathSelector {
    /// Canonical SQL spelling, for diagnostics and EXPLAIN output.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Any => "ANY",
            Self::AnyShortest => "ANY SHORTEST",
            Self::AllShortest => "ALL SHORTEST",
            Self::AnyCheapest => "ANY CHEAPEST",
        }
    }

    /// True for the RaisinDB weighted-path extension (`ANY CHEAPEST`).
    pub fn is_extension(&self) -> bool {
        matches!(self, Self::AnyCheapest)
    }
}

impl std::fmt::Display for PathSelector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Which walks through the graph count as a path.
///
/// RaisinDB's default when none is written is [`PathRestrictor::Acyclic`] —
/// chosen to preserve existing behaviour, because the engine has always skipped
/// already-visited nodes during variable-length traversal. The ISO default
/// could not be confirmed, so `WALK` must be requested explicitly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PathRestrictor {
    /// `WALK` — no distinctness requirement; nodes and edges may repeat.
    Walk,
    /// `TRAIL` — edge-distinct: no edge may be traversed twice.
    Trail,
    /// `ACYCLIC` — node-distinct: no node may be visited twice.
    Acyclic,
}

impl PathRestrictor {
    /// The restrictor applied when a path pattern names none.
    pub const DEFAULT: Self = Self::Acyclic;

    /// Canonical SQL spelling, for diagnostics and EXPLAIN output.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Walk => "WALK",
            Self::Trail => "TRAIL",
            Self::Acyclic => "ACYCLIC",
        }
    }
}

impl std::fmt::Display for PathRestrictor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Which spelling a [`crate::ast::pgq::PathQuantifier`] was written in.
///
/// The two forms occupy disjoint syntactic slots — the legacy form lives
/// *inside* the brackets, the standard form *after* the arrow — so they are
/// lexically disjoint and never ambiguous. They are not, however,
/// interchangeable: legacy `*` means `{1,}` while standard `*` means `{0,}`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum QuantifierSyntax {
    /// Canonical postfix brace form, written after the arrow: `->{1,3}`.
    #[default]
    Standard,
    /// Deprecated Cypher-style form, written inside the brackets: `-[:t*1..3]->`.
    LegacyStar,
}
