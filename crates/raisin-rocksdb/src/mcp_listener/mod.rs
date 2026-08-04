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

//! Holding notification streams open to remote MCP servers.
//!
//! A supervised task, **deliberately not a job**. Every `JobType` carries a hard
//! wall-clock deadline — 600s is the largest in the whole enum — and
//! `TimeoutWatchdog` reaps on *runtime*, not heartbeat staleness, then calls
//! `abort_handle()`. A listener job would therefore have its stream severed on a
//! fixed cycle forever, and would hold one `handler_semaphore` permit the entire
//! time, starving its pool. Every genuinely-forever loop in this codebase is a
//! `tokio::spawn`; this is one of them.
//!
//! One lease per connection (`mcp-listen:{slug}`), matching the discovery lease
//! naming. Streams spread across nodes, and a node dying orphans only its own.
//!
//! Opt-in twice over: the connection must set `refresh_policy.notifications`,
//! and the server must actually offer the guarantee — either by speaking
//! 2026-07-28 or by advertising `capabilities.tools.listChanged`. Holding a
//! socket open to a server that never announces anything is pure cost.

mod supervisor;

pub use supervisor::{spawn, ListenerConfig};
