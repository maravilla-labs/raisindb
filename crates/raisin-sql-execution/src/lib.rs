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

// TODO(v0.2): Update deprecated API usages (geo crate migration) and clean up
#![allow(deprecated)]
#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(unused_variables)]
#![allow(private_interfaces)]

//! RaisinSQL Physical Execution Engine
//!
//! This crate provides the physical execution layer for RaisinDB SQL queries.
//! It contains the physical plan operators, execution engine, and storage integration.
//!
//! This crate depends on tokio, RocksDB, and other heavy dependencies that prevent
//! WASM compilation. The parsing and planning layers are in the `raisin-sql` crate,
//! which is WASM-compatible.
//!
//! # Architecture
//!
//! The physical execution layer consists of several components:
//!
//! 1. **Physical Operators** - Physical plan nodes representing concrete execution strategies
//! 2. **Physical Planner** - Converts logical plans to physical plans with scan selection
//! 3. **Executor** - Execution context and streaming execution engine
//! 4. **Scan Executors** - Physical scans (Table, Prefix, Property, FullText)
//! 5. **Expression Evaluator** - Runtime expression evaluation
//! 6. **Query Engine** - High-level API for complete SQL execution pipeline
//!
//! # Example
//!
//! ```no_run
//! use raisin_sql_execution::QueryEngine;
//! use raisin_rocksdb::RocksDBStorage;
//! use std::sync::Arc;
//!
//! # async fn example() -> Result<(), raisin_error::Error> {
//! let storage = Arc::new(RocksDBStorage::open("./data")?);
//! let engine = QueryEngine::new(storage, "tenant1", "repo1", "main");
//!
//! // Query with automatic workspace-as-table and revision support
//! let mut stream = engine.execute("SELECT * FROM default WHERE __revision = 100").await?;
//!
//! // Process results
//! use futures::StreamExt;
//! while let Some(row) = stream.next().await {
//!     println!("{:?}", row?);
//! }
//! # Ok(())
//! # }
//! ```

pub mod engine;
pub mod physical_plan;

// Re-export commonly used items from raisin-sql
pub use raisin_sql::{
    parse_sql, substitute_params, AnalyzedQuery, AnalyzedStatement, Analyzer, Catalog, DataType,
    LogicalPlan, Optimizer, OptimizerConfig, ParseError, PlanBuilder, PlanError, QueryPlan,
    RaisinDialect, StaticCatalog,
};

// Re-export query engine and batch utilities
pub use engine::{
    batch_requires_async, FunctionInvokeCallback, FunctionInvokeSyncCallback, JobRegistrarCallback,
    QueryEngine, RestoreTreeRegistrarCallback,
};

// Re-export physical plan types
pub use physical_plan::{
    // DML filter classification for sync vs async execution
    classify_filter,
    convert_batch_stream_to_row_stream,
    convert_row_stream_to_batch_stream,
    execute_plan,
    execute_plan_batch,
    from_property_value,
    to_property_value,
    Batch,
    BatchConfig,
    BatchExecutionConfig,
    BatchIterator,
    BatchStream,
    CTEConfig,
    CTEIterator,
    ColumnArray,
    ExecutionContext,
    ExecutionError,
    FilterComplexity,
    IndexCatalog,
    MaterializedCTE,
    NodeIdentifier,
    PhysicalPlan,
    PhysicalPlanner,
    RocksDBIndexCatalog,
    Row,
    RowAccumulator,
    RowStream,
    VectorDistanceMetric,
};
