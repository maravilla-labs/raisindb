// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Thread-local storage for Starlark runtime
//!
//! Manages the thread-local host gateway, logs, and log emitter during
//! execution. The Starlark evaluator is synchronous and its host functions are
//! free functions, so the execution's gateway reaches them through here.

use crate::runtime::host_gateway::HostGateway;
use crate::types::LogEntry;
use std::cell::RefCell;

// Thread-local storage for the gateway, logs, and log emitter during execution
thread_local! {
    /// The execution's host gateway (policy, deadline, stop).
    pub(super) static CURRENT_GATEWAY: RefCell<Option<HostGateway>> = const { RefCell::new(None) };
    static CURRENT_LOGS: RefCell<Vec<LogEntry>> = const { RefCell::new(Vec::new()) };
    static CURRENT_LOG_EMITTER: RefCell<Option<raisin_storage::LogEmitter>> = RefCell::new(None);
}

/// Install the execution's gateway and log emitter on this thread.
pub(super) fn set_thread_api(
    gateway: HostGateway,
    log_emitter: Option<raisin_storage::LogEmitter>,
) {
    CURRENT_GATEWAY.with(|cell| {
        *cell.borrow_mut() = Some(gateway);
    });
    CURRENT_LOG_EMITTER.with(|cell| {
        *cell.borrow_mut() = log_emitter;
    });
}

pub(super) fn clear_thread_api() {
    CURRENT_GATEWAY.with(|cell| {
        *cell.borrow_mut() = None;
    });
    CURRENT_LOG_EMITTER.with(|cell| {
        *cell.borrow_mut() = None;
    });
}

pub(super) fn push_log(entry: LogEntry) {
    // Emit in real-time if emitter is set
    CURRENT_LOG_EMITTER.with(|cell| {
        if let Some(ref emitter) = *cell.borrow() {
            emitter(entry.level.to_string(), entry.message.clone());
        }
    });
    // Buffer the log entry for the final result
    CURRENT_LOGS.with(|cell| {
        cell.borrow_mut().push(entry);
    });
}

/// Take all logs from thread-local storage
pub(super) fn take_logs() -> Vec<LogEntry> {
    CURRENT_LOGS.with(|cell| std::mem::take(&mut *cell.borrow_mut()))
}

/// Clear logs from thread-local storage
pub(super) fn clear_logs() {
    CURRENT_LOGS.with(|cell| {
        cell.borrow_mut().clear();
    });
}
