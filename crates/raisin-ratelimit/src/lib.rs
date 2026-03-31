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

//! Rate limiting implementations for RaisinDB
//!
//! This crate provides concrete implementations of the `RateLimiter` trait
//! from `raisin-context`. The default implementation uses RocksDB for
//! persistent rate limit tracking with a sliding window algorithm.
//!
//! # Features
//!
//! - **rocksdb-backend** (default): RocksDB-backed rate limiter
//!
//! # Examples
//!
//! ## Using the RocksDB Rate Limiter
//!
//! ```rust,no_run
//! use raisin_ratelimit::RocksRateLimiter;
//! use raisin_context::RateLimiter;
//! use std::time::Duration;
//!
//! #[tokio::main]
//! async fn main() {
//!     let limiter = RocksRateLimiter::open("./rate-limit-db").unwrap();
//!
//!     let info = limiter
//!         .check_rate("tenant-123", 100, Duration::from_secs(60))
//!         .await;
//!
//!     if info.allowed {
//!         println!("Request allowed. {} remaining", info.remaining());
//!     } else {
//!         println!("Rate limit exceeded. Try again in {:?}", info.reset_after);
//!     }
//! }
//! ```

#[cfg(feature = "rocksdb-backend")]
mod rocks;

#[cfg(feature = "rocksdb-backend")]
pub use rocks::RocksRateLimiter;

// Re-export the trait and types for convenience
pub use raisin_context::{RateLimitInfo, RateLimiter};
