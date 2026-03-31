// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Timer API implementation for QuickJS runtime
//!
//! This module provides setTimeout/clearTimeout support:
//! - `setTimeout(callback, delay)` - Schedule a callback to run after delay
//! - `clearTimeout(timerId)` - Cancel a pending timer
//! - `setInterval(callback, interval)` - Schedule a repeating callback
//! - `clearInterval(timerId)` - Cancel a repeating timer

mod polyfill;
mod registry;

pub use polyfill::TIMERS_POLYFILL;
pub use registry::TimerRegistry;
