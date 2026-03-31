// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Flow definition types for parsing workflow_data
//!
//! This module defines the core types for flow definitions:
//! - `FlowDefinition` - The complete flow graph with nodes and edges
//! - `FlowNode` - Individual steps in the flow
//! - `StepType` - Enum of all supported step types
//! - Configuration types for AI, decision, function, and human task steps

pub mod config_types;
pub mod definition;
pub mod node_types;

pub use config_types::{
    AIContainerConfig, AiExecutionConfig, DecisionConfig, FunctionStepConfig, HumanTaskConfig,
    TaskOption, TaskType, ToolMode,
};
pub use definition::FlowDefinition;
pub use node_types::{FlowEdge, FlowMetadata, FlowNode, StepType};
