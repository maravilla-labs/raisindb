// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! W3C Fetch API implementation for QuickJS runtime
//!
//! This module provides a full W3C-compliant Fetch API including:
//! - `fetch()` global function
//! - `Request`, `Response`, `Headers` classes
//! - `ReadableStream` for streaming response bodies
//! - `AbortController`/`AbortSignal` for request cancellation
//! - `FormData` for multipart request bodies

mod abort;
mod js_polyfill;
mod stream;
mod types;

pub use abort::AbortRegistry;
pub use js_polyfill::FETCH_POLYFILL;
pub use stream::{StreamReadResult, StreamRegistry};
pub use types::{FetchBody, FetchRequest, FetchResponseMeta};
