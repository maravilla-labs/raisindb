// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Designer format types for parsing workflow_data from the flow designer UI.
//!
//! The designer produces a tree-based format using `node_type` discriminator,
//! while the runtime expects a flat graph format with `step_type` and `edges`.
//! This module handles parsing the designer format and converting it to runtime format.

pub mod config_types;
mod conversion;
pub mod types;

#[cfg(test)]
mod tests;

pub use config_types::{
    ChatStepConfig, ChatTerminationConfig, DesignerAiConfig, DesignerAiErrorBehavior,
    DesignerToolMode, HandoffTarget,
};
pub use types::{
    DesignerContainerRule, DesignerContainerType, DesignerErrorStrategy, DesignerFlowDefinition,
    DesignerNode, DesignerStepProperties, DesignerStepType, ExecutionIdentityMode, RaisinReference,
    RetryConfig, StepErrorBehavior,
};
