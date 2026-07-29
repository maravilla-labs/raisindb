// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Types for parallel execution (stubs for future implementation)

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Request to create a child flow for parallel execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateChildFlowRequest {
    pub branch_id: String,
    pub parent_instance_id: String,
    /// Parent step that forked this branch. Recorded on the child so its
    /// completion notification can name the step to resume, which matters
    /// once a flow has more than one parked container.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_step_id: Option<String>,
    pub flow_definition: Value,
    pub input: Value,
}

/// Status of a child flow in parallel execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChildFlowStatus {
    pub branch_id: String,
    pub instance_id: String,
    pub status: String,
    pub output: Option<Value>,
    pub error: Option<String>,
}
