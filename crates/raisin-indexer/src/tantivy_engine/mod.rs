// SPDX-License-Identifier: BSL-1.1

//! Tantivy-based full-text search indexing engine for RaisinDB.

mod batch;
mod document;
mod index_manager;
mod indexing_impl;
mod language;
mod properties;
mod query;
mod schema;
mod search;
mod types;
mod utils;

pub use types::{BatchIndexContext, TantivyIndexingEngine};
