//! SQL Completion Engine
//!
//! Context-aware SQL completions for IDE integration. Provides semantic
//! suggestions based on SQL context, table catalog, and function registry.

mod context;
mod partial;
mod provider;
mod types;

pub use context::{AnalyzedContext, SqlContext, TableAlias};
pub use provider::CompletionProvider;
pub use types::{CompletionItem, CompletionKind, CompletionResult, InsertTextFormat};
