//! Function analysis
//!
//! This module handles the analysis of SQL function calls including:
//! - Regular scalar functions
//! - Aggregate functions
//! - Window functions (with OVER clause)
//! - Special functions like COALESCE, TO_JSON, JSON_GET, EMBEDDING

mod analysis;
mod constant_fold;
mod window;
