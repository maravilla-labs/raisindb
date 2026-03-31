/**
 * Flow execution types for the RaisinDB JS SDK.
 *
 * These types mirror the backend FlowEvent / FlowExecutionEvent enums
 * used for real-time SSE streaming of flow step-level events.
 */

// ============================================================================
// Flow Instance
// ============================================================================

/** Response returned when a flow is started via the run endpoint. */
export interface FlowRunResponse {
  /** The created flow instance ID */
  instance_id: string;
  /** Job ID for tracking execution */
  job_id: string;
  /** Status (always "queued" for async execution) */
  status: string;
}

/** Flow instance status values */
export type FlowInstanceStatus =
  | 'queued'
  | 'pending'
  | 'running'
  | 'completed'
  | 'failed'
  | 'waiting'
  | 'cancelled';

/** Response from GET flow instance status endpoint. */
export interface FlowInstanceStatusResponse {
  /** Flow instance ID */
  id: string;
  /** Current status */
  status: FlowInstanceStatus;
  /** Flow-scoped variables */
  variables: Record<string, unknown>;
  /** Path to the flow definition */
  flow_path: string;
  /** ISO 8601 timestamp of when the flow started */
  started_at: string;
  /** Error message (if status is failed) */
  error?: string;
}

// ============================================================================
// Flow Execution Events (matching backend FlowEvent enum)
// ============================================================================

/** Base properties shared by all events */
interface BaseFlowEvent {
  timestamp: string;
}

/** A step has started execution */
export interface StepStartedEvent extends BaseFlowEvent {
  type: 'step_started';
  node_id: string;
  step_name?: string;
  step_type: string;
}

/** A step has completed successfully */
export interface StepCompletedEvent extends BaseFlowEvent {
  type: 'step_completed';
  node_id: string;
  output: unknown;
  duration_ms: number;
}

/** A step has failed */
export interface StepFailedEvent extends BaseFlowEvent {
  type: 'step_failed';
  node_id: string;
  error: string;
  duration_ms: number;
}

/** Flow is waiting for external input (human task, approval, etc.) */
export interface FlowWaitingEvent extends BaseFlowEvent {
  type: 'flow_waiting';
  node_id: string;
  wait_type: string;
  reason: string;
}

/** Flow has resumed after waiting */
export interface FlowResumedEvent extends BaseFlowEvent {
  type: 'flow_resumed';
  node_id: string;
  wait_duration_ms: number;
}

/** Flow has completed successfully */
export interface FlowCompletedEvent extends BaseFlowEvent {
  type: 'flow_completed';
  output: unknown;
  total_duration_ms: number;
}

/** Flow has failed */
export interface FlowFailedEvent extends BaseFlowEvent {
  type: 'flow_failed';
  error: string;
  failed_at_node?: string;
  total_duration_ms: number;
}

/** Partial text content from AI streaming */
export interface TextChunkEvent extends BaseFlowEvent {
  type: 'text_chunk';
  text: string;
}

/** AI tool call started */
export interface ToolCallStartedEvent extends BaseFlowEvent {
  type: 'tool_call_started';
  tool_call_id: string;
  function_name: string;
  arguments: unknown;
}

/** AI tool call completed */
export interface ToolCallCompletedEvent extends BaseFlowEvent {
  type: 'tool_call_completed';
  tool_call_id: string;
  result: unknown;
  /** Error message if the tool call failed */
  error?: string;
  /** Execution duration in milliseconds */
  duration_ms?: number;
}

/** Partial thinking/reasoning content from AI streaming */
export interface ThoughtChunkEvent extends BaseFlowEvent {
  type: 'thought_chunk';
  text: string;
}

/** AI conversation node was created or resolved */
export interface ConversationCreatedEvent extends BaseFlowEvent {
  type: 'conversation_created';
  conversation_path: string;
  workspace: string;
}

/** An AI message was persisted to the node tree */
export interface MessageSavedEvent extends BaseFlowEvent {
  type: 'message_saved';
  message_path: string;
  role: string;
  conversation_path: string;
}

/** Log message from flow execution */
export interface LogEvent extends BaseFlowEvent {
  type: 'log';
  level: string;
  message: string;
  node_id?: string;
}

/** Union of all flow execution events streamed via SSE */
export type FlowExecutionEvent =
  | StepStartedEvent
  | StepCompletedEvent
  | StepFailedEvent
  | FlowWaitingEvent
  | FlowResumedEvent
  | FlowCompletedEvent
  | FlowFailedEvent
  | TextChunkEvent
  | ToolCallStartedEvent
  | ToolCallCompletedEvent
  | ThoughtChunkEvent
  | ConversationCreatedEvent
  | MessageSavedEvent
  | LogEvent;

/** Type guard: is this a terminal event (flow_completed or flow_failed)? */
export function isTerminalEvent(
  event: FlowExecutionEvent,
): event is FlowCompletedEvent | FlowFailedEvent {
  return event.type === 'flow_completed' || event.type === 'flow_failed';
}
