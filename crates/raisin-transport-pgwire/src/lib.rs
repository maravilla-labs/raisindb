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

//! # RaisinDB PostgreSQL Wire Protocol Transport
//!
//! This crate provides a PostgreSQL wire protocol (pgwire) transport layer for RaisinDB,
//! allowing PostgreSQL clients to connect to and query RaisinDB using the standard
//! PostgreSQL protocol.
//!
//! ## Features
//!
//! - Full PostgreSQL wire protocol support via `pgwire` crate
//! - Authentication support (password-based and no-auth modes)
//! - Simple query protocol (text-based queries)
//! - Extended query protocol (prepared statements with binary encoding)
//! - Type mapping between PostgreSQL types and RaisinDB types
//! - Result encoding for various data types
//! - Comprehensive error handling and reporting
//!
//! ## Usage
//!
//! ```rust,ignore
//! // Example usage (types not yet implemented)
//! use raisin_transport_pgwire::{PgWireServer, PgWireConfig};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let config = PgWireConfig::default()
//!         .with_host("127.0.0.1")
//!         .with_port(5432);
//!
//!     let server = PgWireServer::new(config)?;
//!     server.start().await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Architecture
//!
//! The crate is organized into the following modules:
//!
//! - `server`: Core server implementation and lifecycle management
//! - `auth`: Authentication handlers and mechanisms
//! - `simple_query`: Simple query protocol implementation
//! - `extended_query`: Extended query protocol with prepared statements
//! - `type_mapping`: PostgreSQL to RaisinDB type conversions
//! - `result_encoder`: Encoding RaisinDB results to PostgreSQL format
//! - `error`: Error types and conversions

// TODO(v0.2): Clean up unused code
#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(unused_variables)]

// Module declarations
pub mod auth;
pub mod error;
pub mod extended_query;
pub mod result_encoder;
pub mod server;
pub mod simple_query;
pub mod type_mapping;
pub mod type_mapping_binary;

// Re-exports
pub use auth::{ApiKeyValidator, ConnectionContext, RaisinAuthHandler};
pub use error::{PgWireTransportError, Result};
pub use extended_query::{RaisinExtendedQueryHandler, RaisinQueryParser, RaisinStatement};
pub use result_encoder::{infer_schema_from_rows, ColumnInfo, ResultEncoder};
pub use server::{PgWireConfig, PgWireConfigBuilder, PgWireServer};
pub use simple_query::RaisinSimpleQueryHandler;
