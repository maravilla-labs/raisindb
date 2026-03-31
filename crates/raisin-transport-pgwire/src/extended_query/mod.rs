// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Extended query protocol handler for prepared statements and binary protocol.
//!
//! This module implements the PostgreSQL extended query protocol, which allows
//! clients to prepare statements, bind parameters (in binary or text format),
//! and execute queries with better performance and type safety.
//!
//! # Architecture
//!
//! The extended query protocol flow:
//!
//! 1. **Parse**: Client sends SQL with placeholders ($1, $2, etc.)
//! 2. **Describe Statement**: Client can request parameter and result types
//! 3. **Bind**: Client binds actual parameter values to a portal
//! 4. **Describe Portal**: Client can request result column information
//! 5. **Execute**: Client executes the portal with bound parameters
//!
//! # Submodules
//!
//! - [`statement`]: Prepared statement types and SQL query parser
//! - [`params`]: Parameter binding and extraction
//! - [`schema`]: SQL schema inference via the analyzer
//! - [`session`]: SET / SHOW / RESET / USE BRANCH handlers
//! - [`handler`]: `ExtendedQueryHandler` trait implementation (dispatch)
//!
//! # Example
//!
//! ```rust,ignore
//! use raisin_transport_pgwire::extended_query::RaisinExtendedQueryHandler;
//!
//! let handler = RaisinExtendedQueryHandler::new(storage, auth_handler);
//! // Handler is used by pgwire server automatically
//! ```

mod handler;
mod params;
mod schema;
mod session;
mod statement;

// Re-export public API -- same paths as before the split.
pub use statement::{RaisinQueryParser, RaisinStatement};

use crate::auth::{ApiKeyValidator, RaisinAuthHandler};
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::Storage;
use std::sync::Arc;

/// Extended query handler for RaisinDB.
///
/// This handler processes prepared statements with parameter binding,
/// executes queries through the RaisinDB query engine, and returns
/// results in the PostgreSQL wire format.
///
/// Unlike a static query engine approach, this handler creates a QueryEngine
/// dynamically for each query using the connection's tenant/repository context.
pub struct RaisinExtendedQueryHandler<S, V, P>
where
    S: Storage + TransactionalStorage + 'static,
    V: ApiKeyValidator,
    P: pgwire::api::auth::ServerParameterProvider,
{
    /// Storage backend for data access
    pub(crate) storage: Arc<S>,
    /// Authentication handler to retrieve connection context
    pub(crate) auth_handler: Arc<RaisinAuthHandler<V, P>>,
    /// Parser for analyzing SQL statements
    pub(crate) query_parser: Arc<RaisinQueryParser>,
    /// Optional Tantivy indexing engine for full-text search
    #[cfg(feature = "indexing")]
    pub(crate) indexing_engine: Option<Arc<raisin_indexer::TantivyIndexingEngine>>,
    /// Optional HNSW engine for vector similarity search
    #[cfg(feature = "indexing")]
    pub(crate) hnsw_engine: Option<Arc<raisin_hnsw::HnswIndexingEngine>>,
}

impl<S, V, P> RaisinExtendedQueryHandler<S, V, P>
where
    S: Storage + TransactionalStorage + 'static,
    V: ApiKeyValidator,
    P: pgwire::api::auth::ServerParameterProvider,
{
    /// Create a new extended query handler.
    ///
    /// # Arguments
    ///
    /// * `storage` - Storage backend for data access
    /// * `auth_handler` - Authentication handler to retrieve connection context
    pub fn new(storage: Arc<S>, auth_handler: Arc<RaisinAuthHandler<V, P>>) -> Self {
        Self {
            storage,
            auth_handler,
            query_parser: Arc::new(RaisinQueryParser::new()),
            #[cfg(feature = "indexing")]
            indexing_engine: None,
            #[cfg(feature = "indexing")]
            hnsw_engine: None,
        }
    }

    /// Set the Tantivy indexing engine for full-text search support.
    #[cfg(feature = "indexing")]
    pub fn with_indexing_engine(
        mut self,
        engine: Arc<raisin_indexer::TantivyIndexingEngine>,
    ) -> Self {
        self.indexing_engine = Some(engine);
        self
    }

    /// Set the HNSW engine for vector similarity search support.
    #[cfg(feature = "indexing")]
    pub fn with_hnsw_engine(mut self, engine: Arc<raisin_hnsw::HnswIndexingEngine>) -> Self {
        self.hnsw_engine = Some(engine);
        self
    }
}
