// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Type definitions for the flow runtime

pub mod callbacks;
pub mod compensation;
pub mod context;
pub mod designer_format;
pub mod events;
pub mod flow_definition;
pub mod flow_instance;
pub mod parallel;
pub mod result;
pub mod step_execution;
pub mod test_config;

// Re-export commonly used types
pub use callbacks::{AiCallContext, FlowCallbacks};
pub use compensation::{CompensationEntry, CompensationStatus};
pub use context::{
    ContextFrame, FlowContext, FlowContextError, FrameType, TriggerEventType, TriggerInfo,
};
pub use events::FlowExecutionEvent;
pub use flow_definition::{
    AIContainerConfig, AiExecutionConfig, DecisionConfig, FlowDefinition, FlowEdge, FlowMetadata,
    FlowNode, FunctionStepConfig, HumanTaskConfig, StepType, TaskOption, TaskType, ToolMode,
};
pub use flow_instance::{FlowInstance, FlowMetrics, FlowStatus, WaitInfo, WaitType};
pub use parallel::{ChildFlowStatus, CreateChildFlowRequest};
pub use result::{FlowError, FlowResult, StepResult};
pub use step_execution::{FlowStepExecution, StepStatus};
pub use test_config::{FunctionMock, MockBehavior, TestRunConfig};
