// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Flow runtime execution engine.
//!
//! This module provides the core execution loop for flows:
//! - `executor` - Main execution loop with hybrid batching
//! - `state_manager` - State persistence with OCC
//! - `subscription` - Wait subscription management
//! - `compensation` - Saga rollback logic
//! - `retry` - Retry logic with exponential backoff
//! - `timeout` - Timeout handling for flow operations

pub mod compensation;
pub mod data_mapper;
pub mod executor;
pub mod resume;
pub mod retry;
pub mod state_manager;
pub mod subscription;
pub mod timeout;

// Re-export commonly used functions
pub use compensation::{push_compensation, rollback_flow};
pub use data_mapper::DataMapper;
pub use executor::execute_flow;
pub use resume::resume_flow;
pub use retry::{strategies, RetryConfig};
pub use state_manager::{
    create_flow_instance, load_instance, save_instance, save_instance_with_version,
};
pub use subscription::SubscriptionRegistry;
pub use timeout::{with_optional_timeout, with_timeout, TimedResult, TimeoutConfig};
