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

//! RaisinGraph Algorithms
//!
//! This crate implements the "Graph Projection" engine for RaisinDB.
//! It allows loading subgraphs from RocksDB into an optimized in-memory structure
//! (CSR - Compressed Sparse Row) to run global graph algorithms like PageRank,
//! Community Detection, and Centrality measures.
//!
//! # Architecture
//!
//! 1. **Projection**: Scan RocksDB (or use Path Index) to find relevant nodes/edges.
//! 2. **Mapping**: Map string IDs (UUIDs/Paths) to dense integers (u32).
//! 3. **CSR Construction**: Build a `petgraph::csr::Csr` graph.
//! 4. **Execution**: Run algorithms using `rayon` for parallelism.
//! 5. **Writeback**: Map results back to string IDs and update Node Properties.

pub mod algorithms;
pub mod error;
pub mod projection;
pub mod writeback;

pub use error::GraphError;
pub use projection::GraphProjection;
