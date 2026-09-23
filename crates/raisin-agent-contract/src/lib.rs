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

//! The wire contract between the AgentRun runtime and a domain reducer.
//!
//! A domain reducer is a pure function `(persisted state, authoritative event)
//! -> (next state, effects[])`. Core owns durability, sequencing, leases and
//! control; the reducer owns domain meaning. This crate is the ONE place the
//! envelope shapes and their validation rules live, so core, the WebAssembly
//! adapter and every guest reducer check the same rules with the same code.
//!
//! # Versioning
//!
//! - [`CONTRACT_V1`] names the reducer envelope, [`TOOL_RESULT_V1`] the
//!   tool-result envelope.
//! - Within `/1`, new OPTIONAL fields may be added; both sides ignore unknown
//!   fields (nothing here uses `deny_unknown_fields`).
//! - The event-kind and effect-kind sets are CLOSED per version. A new kind is
//!   `/2`.
//!
//! The ABI underneath is unchanged: a reducer is a named handler of the frozen
//! `raisin:function` world taking and returning JSON.

#![warn(missing_docs)]

pub mod canonical;
pub mod projection;
pub mod reducer;
pub mod tool_result;
pub mod validate;

pub use canonical::canonical_json;
pub use projection::{effective_projection, ItemStatus, PlanItem, PlanProjection};
pub use reducer::*;
pub use tool_result::*;
pub use validate::{validate_response, validate_tool_result, Refusal};

/// Contract id of the reducer request/response envelope, version 1.
pub const CONTRACT_V1: &str = "raisin.agent-run.reducer/1";

/// Envelope id of a tool result, version 1.
pub const TOOL_RESULT_V1: &str = "raisin.tool-result/1";

/// Largest canonical-JSON size of a reducer state (rule R9). Larger outputs go
/// through a result reference, never through the state.
pub const MAX_STATE_BYTES: usize = 256 * 1024;

pub mod fixtures;
