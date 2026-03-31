// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! JavaScript polyfill for the W3C Fetch API
//!
//! This module contains the JavaScript code that creates the W3C-compliant
//! Fetch API surface, split by API category:
//!
//! - [`headers`] - DOMException and Headers class
//! - [`abort`] - AbortSignal and AbortController classes
//! - [`encoding`] - TextDecoder, TextEncoder, atob/btoa
//! - [`streams`] - ReadableStream and ReadableStreamDefaultReader
//! - [`request`] - Request class
//! - [`response`] - Response class
//! - [`fetch_fn`] - FormData class and fetch() function

mod abort;
mod encoding;
mod fetch_fn;
mod headers;
mod request;
mod response;
mod streams;

use std::sync::LazyLock;

/// Combined JavaScript code that creates the full W3C Fetch API.
///
/// This is evaluated in the QuickJS context to set up:
/// - `fetch()` global function
/// - `Headers` class
/// - `Request` class
/// - `Response` class
/// - `ReadableStream` class (simplified)
/// - `AbortController` / `AbortSignal` classes
/// - `FormData` class
pub static FETCH_POLYFILL: LazyLock<String> = LazyLock::new(|| {
    [
        headers::JS_HEADERS,
        abort::JS_ABORT,
        encoding::JS_ENCODING,
        streams::JS_STREAMS,
        request::JS_REQUEST,
        response::JS_RESPONSE,
        fetch_fn::JS_FETCH_FN,
    ]
    .join("\n")
});
