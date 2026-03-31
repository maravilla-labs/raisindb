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

//! Job management system for background tasks
//!
//! This module provides a centralized job management system that works
//! across all storage implementations.

pub mod conversation_events;
pub mod flow_events;
pub mod monitor;
pub mod persistence;
pub mod pool;
pub mod registry;
pub mod types;
pub mod worker;

pub use conversation_events::{
    global_conversation_broadcaster, ConversationEvent, ConversationEventBroadcaster,
};
pub use flow_events::{global_flow_broadcaster, FlowEvent, FlowEventBroadcaster};
pub use monitor::{JobEvent, JobLogEntry, JobMonitor, JobMonitorHub, LogEmitter, LoggingMonitor};
pub use persistence::JobPersistence;
pub use pool::{CategoryPoolStats, WorkerPool, WorkerPoolStats};
pub use registry::{global_registry, JobRegistry};
pub use types::{
    AssetProcessingOptions, BatchIndexOperation, IndexOperation, JobCategory, JobContext,
    JobHandle, JobId, JobInfo, JobPriority, JobStatus, JobType, PdfExtractionStrategy,
};
pub use worker::JobWorker;
