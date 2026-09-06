// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! TypeScript guest-SDK backend.
//!
//! The TS SDK's design goal is that a QuickJS function componentizes with ZERO
//! source changes, so it does not re-implement the `raisin.*` surface: it ships
//! `quickjs/api_wrapper.js` VERBATIM and defines `__raisin_call` over the WIT
//! gateway. Copying the wrapper rather than regenerating it is what keeps the
//! frozen per-method error conventions identical between the two runtimes.

/// The QuickJS API wrapper, compiled into this binary so the copy in the TS SDK
/// can never drift from the runtime's own.
pub const API_WRAPPER_JS: &str = include_str!("../../quickjs/api_wrapper.js");

/// `sdks/ts/function-wasm/src/generated/api_wrapper.js` — a byte-identical copy.
pub fn render_api_wrapper() -> String {
    API_WRAPPER_JS.to_string()
}

/// `sdks/ts/function-wasm/src/generated/raisin.d.ts`.
pub fn render_dts() -> String {
    super::dts::render_sdk_dts()
}
