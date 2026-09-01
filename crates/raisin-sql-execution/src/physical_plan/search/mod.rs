//! `HYBRID_SEARCH`, `FULLTEXT_SEARCH` and `KNN`: one implementation.
//!
//! | file | owns |
//! |---|---|
//! | [`scope`] | `WorkspaceSet`, the `workspaces =>` grammar, scope resolution |
//! | [`vector_of`] | `VECTOR_OF(...)`: a node's own stored vector, and self-exclusion |
//! | [`args`] | the positional + named argument grammar for all three |
//! | [`legs`] | full-text and vector leg dispatch, push-down construction |
//! | [`fusion`] | weighted RRF and the total-order sort |
//! | [`emit`] | the fetch -> RLS -> residual -> yield loop |
//!
//! # The rule
//!
//! **The universe is an argument; everything else is `WHERE`.**
//!
//! `workspaces` is an argument because the universe changes what top-k MEANS:
//! `workspaces => 'a, b' LIMIT 10` returns the ten best rows in a and b, while
//! `'ALL READABLE' ... WHERE workspace_id IN ('a','b') LIMIT 10` returns
//! whichever of a's and b's rows survived the GLOBAL best-N -- which can be
//! empty while matching documents exist. `node_type`, `path`, `name`,
//! `properties->>'k'`, `workspace_id` and `vector_distance` are columns, and they
//! are filtered with `WHERE`. There is no `node_types =>` and no `path_prefix =>`
//! argument.
//!
//! # Push-down is an optimisation; the residual filter is the correctness
//!
//! Straight from the `REFERENCES(...)` discipline in CLAUDE.md. The workspace
//! set and the `shape_types` term narrow what the indexes are asked for. The
//! `Filter` the plan builder puts above the table function is what makes the
//! answer right. A push-down that under-narrows costs candidates; the
//! `shape_types` one deliberately OVER-matches (the index field is multi-valued
//! and holds node_type + archetype + nested element_type) and is sound only
//! because of that residual. Deleting the residual as "already covered by the
//! push-down" leaks archetype-only matches into `WHERE node_type = 'X'`.

pub mod args;
pub mod chunk_text;
pub mod emit;
pub mod fusion;
pub mod legs;
pub mod scope;
pub mod vector_of;

/// How much wider than `limit` each leg is drawn when anything can drop rows
/// after ranking.
///
/// ONE constant, shared with
/// `planner::plan_dispatch::vector_knn::RESIDUAL_OVERFETCH`, which re-exports
/// this rather than declaring its own. RLS is a residual filter; same problem,
/// same number. Two constants with one job in two files is how
/// `DEFAULT_MAX_DISTANCE = 0.6` ended up declared twice in a single file.
pub const SEARCH_OVERFETCH: usize = 20;

/// Hard ceiling on what either leg is ever asked for, including after a
/// re-draw. `limit` is validated `1..=1000`, so `limit * SEARCH_OVERFETCH` can
/// reach 20 000 without it.
pub const SEARCH_LEG_CAP: usize = 2000;

/// The Reciprocal Rank Fusion constant.
///
/// NOT exposed as an argument, and that is deliberate rather than an oversight:
///
/// * nobody can tune it without a labelled relevance set, and anyone who has one
///   is doing offline evaluation, not writing a query;
/// * it is GLOBAL. Varying it per query makes two callers' scores
///   incomparable -- including one agent's scores across two turns, which is
///   what breaks a rerank-and-threshold loop;
/// * exposing it invites cargo-cult tuning that HIDES real bugs. Someone whose
///   vector leg is dead will find that lowering `k` "improves" results and never
///   look for the fault. That is not hypothetical: the dead vector leg in this
///   very code presented as plausible ranking with a NULL `vector_rank` and
///   nothing logged.
pub const RRF_K: f64 = 60.0;
