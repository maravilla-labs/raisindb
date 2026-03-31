//! Security module for RLS (Row-Level Security) enforcement.
//!
//! This module provides:
//! - Path pattern matching for permission path rules
//! - Condition evaluation for dynamic permission checks
//! - Permission checking for read/write operations
//! - Field-level filtering for sensitive data
//! - REL context building for expression evaluation
//! - Graph-based relationship resolution for RELATES conditions

mod condition_evaluator;
mod field_filter;
mod graph_resolver;
mod path_matcher;
mod permission_checker;
mod rel_context;

pub use condition_evaluator::ConditionEvaluator;
pub use field_filter::filter_node_fields;
pub use graph_resolver::RocksDBGraphResolver;
pub use path_matcher::{matches_path_pattern, PathMatcher};
pub use permission_checker::PermissionChecker;
pub use rel_context::build_rel_context;
