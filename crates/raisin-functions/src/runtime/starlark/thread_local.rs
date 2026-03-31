// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Thread-local storage for Starlark runtime
//!
//! Manages thread-local API, handle, logs, and log emitter during execution.

use crate::api::FunctionApi;
use crate::types::LogEntry;
use std::cell::RefCell;
use std::sync::Arc;
use tokio::runtime::Handle;

// Thread-local storage for the API, Handle, Logs, and LogEmitter during execution
thread_local! {
    pub(super) static CURRENT_API: RefCell<Option<Arc<dyn FunctionApi>>> = RefCell::new(None);
    pub(super) static CURRENT_HANDLE: RefCell<Option<Handle>> = const { RefCell::new(None) };
    static CURRENT_LOGS: RefCell<Vec<LogEntry>> = const { RefCell::new(Vec::new()) };
    static CURRENT_LOG_EMITTER: RefCell<Option<raisin_storage::LogEmitter>> = RefCell::new(None);
}

/// Set the current API for the thread
pub(super) fn set_thread_api(
    api: Arc<dyn FunctionApi>,
    handle: Handle,
    log_emitter: Option<raisin_storage::LogEmitter>,
) {
    CURRENT_API.with(|cell| {
        *cell.borrow_mut() = Some(api);
    });
    CURRENT_HANDLE.with(|cell| {
        *cell.borrow_mut() = Some(handle);
    });
    CURRENT_LOG_EMITTER.with(|cell| {
        *cell.borrow_mut() = log_emitter;
    });
}

/// Clear the current API for the thread
pub(super) fn clear_thread_api() {
    CURRENT_API.with(|cell| {
        *cell.borrow_mut() = None;
    });
    CURRENT_HANDLE.with(|cell| {
        *cell.borrow_mut() = None;
    });
    CURRENT_LOG_EMITTER.with(|cell| {
        *cell.borrow_mut() = None;
    });
}

/// Push a log entry to thread-local storage and emit in real-time
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
