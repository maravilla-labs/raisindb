//! Flow event conversion utilities
//!
//! Converts FlowExecutionEvent from the flow runtime into FlowEvent
//! for SSE broadcasting to clients.

/// Convert a FlowExecutionEvent from the runtime to a FlowEvent for SSE broadcasting
pub(super) fn convert_flow_event(
    event: raisin_flow_runtime::types::FlowExecutionEvent,
) -> raisin_storage::jobs::FlowEvent {
    use raisin_flow_runtime::types::FlowExecutionEvent;
    use raisin_storage::jobs::FlowEvent;

    match event {
        FlowExecutionEvent::StepStarted {
            node_id,
            step_name,
            step_type,
            timestamp,
        } => FlowEvent::StepStarted {
            node_id,
            step_name,
            step_type,
            timestamp: timestamp.to_rfc3339(),
        },
        FlowExecutionEvent::StepCompleted {
            node_id,
            output,
            duration_ms,
            timestamp,
        } => FlowEvent::StepCompleted {
            node_id,
            output,
            duration_ms,
            timestamp: timestamp.to_rfc3339(),
        },
        FlowExecutionEvent::StepFailed {
            node_id,
            error,
            duration_ms,
            timestamp,
        } => FlowEvent::StepFailed {
            node_id,
            error,
            duration_ms,
            timestamp: timestamp.to_rfc3339(),
        },
        FlowExecutionEvent::FlowWaiting {
            node_id,
            wait_type,
            reason,
            timestamp,
        } => FlowEvent::FlowWaiting {
            node_id,
            wait_type,
            reason,
            timestamp: timestamp.to_rfc3339(),
        },
        FlowExecutionEvent::FlowResumed {
            node_id,
            wait_duration_ms,
            timestamp,
        } => FlowEvent::FlowResumed {
            node_id,
            wait_duration_ms,
            timestamp: timestamp.to_rfc3339(),
        },
        FlowExecutionEvent::FlowCompleted {
            output,
            total_duration_ms,
            timestamp,
        } => FlowEvent::FlowCompleted {
            output,
            total_duration_ms,
            timestamp: timestamp.to_rfc3339(),
        },
        FlowExecutionEvent::FlowFailed {
            error,
            failed_at_node,
            total_duration_ms,
            timestamp,
        } => FlowEvent::FlowFailed {
            error,
            failed_at_node,
            total_duration_ms,
            timestamp: timestamp.to_rfc3339(),
        },
        FlowExecutionEvent::Log {
            level,
            message,
            node_id,
            timestamp,
        } => FlowEvent::Log {
            level,
            message,
            node_id,
            timestamp: timestamp.to_rfc3339(),
        },
        FlowExecutionEvent::TextChunk { text, timestamp } => FlowEvent::TextChunk {
            text,
            timestamp: timestamp.to_rfc3339(),
        },
        FlowExecutionEvent::ToolCallStarted {
            tool_call_id,
            function_name,
            arguments,
            timestamp,
        } => FlowEvent::ToolCallStarted {
            tool_call_id,
            function_name,
            arguments,
            timestamp: timestamp.to_rfc3339(),
        },
        FlowExecutionEvent::ToolCallCompleted {
            tool_call_id,
            result,
            error,
            duration_ms,
            timestamp,
        } => FlowEvent::ToolCallCompleted {
            tool_call_id,
            result,
            error,
            duration_ms,
            timestamp: timestamp.to_rfc3339(),
        },
        FlowExecutionEvent::ThoughtChunk { text, timestamp } => FlowEvent::ThoughtChunk {
            text,
            timestamp: timestamp.to_rfc3339(),
        },
        FlowExecutionEvent::ConversationCreated {
            conversation_path,
            workspace,
            timestamp,
        } => FlowEvent::ConversationCreated {
            conversation_path,
            workspace,
            timestamp: timestamp.to_rfc3339(),
        },
        FlowExecutionEvent::MessageSaved {
            message_path,
            role,
            conversation_path,
            timestamp,
        } => FlowEvent::MessageSaved {
            message_path,
            role,
            conversation_path,
            timestamp: timestamp.to_rfc3339(),
        },
    }
}
