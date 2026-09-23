// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The shared conformance fixtures, and the one routine that checks them.
//!
//! Core and every guest reducer run the SAME files through the SAME check, and
//! both assert the file COUNT, so a new fixture cannot be silently skipped on
//! either side.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::reducer::{ReducerRequest, ReducerResponse};
use crate::tool_result::ToolResultEnvelope;
use crate::validate::{validate_response, validate_tool_result};

/// Directory holding `reducer/v1/*.json` and `tool-result/v1/*.json`.
pub const FIXTURES_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures");

/// Number of reducer fixtures in `reducer/v1`.
pub const REDUCER_FIXTURE_COUNT: usize = 17;

/// Number of tool-result fixtures in `tool-result/v1`.
pub const TOOL_RESULT_FIXTURE_COUNT: usize = 4;

/// Every `*.json` file of `dir`, sorted by name.
pub fn fixture_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("reading {}: {e}", dir.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    files
}

/// Outcome of one fixture: `"ok"` or the refusal code.
fn outcome(result: Result<(), crate::Refusal>) -> String {
    match result {
        Ok(()) => "ok".to_owned(),
        Err(r) => r.code,
    }
}

/// Check one reducer fixture `{request, response, expect}`.
///
/// Returns `Err` with a description when the observed outcome differs.
pub fn check_reducer_fixture(fixture: &Value) -> Result<(), String> {
    let req: ReducerRequest = serde_json::from_value(fixture["request"].clone())
        .map_err(|e| format!("request does not decode: {e}"))?;
    let resp: ReducerResponse = serde_json::from_value(fixture["response"].clone())
        .map_err(|e| format!("response does not decode: {e}"))?;
    let expect = fixture["expect"].as_str().unwrap_or_default();
    let got = outcome(validate_response(&req, &resp));
    if got == expect {
        Ok(())
    } else {
        Err(format!("expected '{expect}', got '{got}'"))
    }
}

/// Check one tool-result fixture `{envelope, expect}`.
pub fn check_tool_result_fixture(fixture: &Value) -> Result<(), String> {
    let env: ToolResultEnvelope = serde_json::from_value(fixture["envelope"].clone())
        .map_err(|e| format!("envelope does not decode: {e}"))?;
    let expect = fixture["expect"].as_str().unwrap_or_default();
    let got = outcome(validate_tool_result(&env));
    if got == expect {
        Ok(())
    } else {
        Err(format!("expected '{expect}', got '{got}'"))
    }
}
