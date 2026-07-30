//! The matched-path value bound to a PGQ path variable.
//!
//! The type itself is engine-neutral and lives in
//! [`crate::physical_plan::graph_algo::path`] so the Cypher and SQL/PGQ engines
//! share one path representation and one set of path algorithms. This module
//! only re-exports it under the name PGQ matching code refers to.
//!
//! Rendering a path into SQL values (`nodes(p)`, `edges(p)`, `element_id(p)`, …)
//! is a projection concern and lives in
//! [`crate::physical_plan::pgq::filter`]'s `path_accessors` module — there is no
//! PATH column type, so a path only ever reaches a client through an accessor.

pub use crate::physical_plan::graph_algo::path::{GraphPath, PathEdge, PathNode};
