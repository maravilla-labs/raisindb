// SPDX-License-Identifier: BSL-1.1

//! Abstract Syntax Tree types for Cypher queries

pub mod expr;
pub mod pattern;
pub mod statement;

pub use expr::{BinOp, Expr, Literal, UnOp};
pub use pattern::{
    Direction, GraphPattern, NodePattern, PathPattern, PatternElement, Range, RelPattern,
};
pub use statement::{Clause, Order, OrderBy, Query, RemoveItem, ReturnItem, SetItem, Statement};

/// Span information for source locations
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub line: usize,
    pub column: usize,
}

impl Span {
    pub fn new(start: usize, end: usize, line: usize, column: usize) -> Self {
        Self {
            start,
            end,
            line,
            column,
        }
    }
}
