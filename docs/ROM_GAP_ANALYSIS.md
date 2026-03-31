# ROM Gap Analysis (Current State -> Robust ROM Runtime)

This document maps the current implementation to ROM requirements and lists
the gaps required to run ROM workflows robustly end-to-end.

## Current Assets

- `raisin:Flow`, `raisin:FlowInstance`, `raisin:FlowStepExecution` node types.
- `raisin-flow-runtime` executes a flow graph with steps and async waits.
- Decision routing via REL conditions in flow handlers.
- Messaging system with inbox/outbox routing and WebSocket delivery.
- `raisin:AIAgent` for agent configuration, `raisin:AIActor` and `raisin:HumanActor`
  for actor registry, `raisin:Message` for human I/O.

## Primary Gaps

### 1) ROM Schema and Validation
Current:
- `raisin:Flow.workflow_data` describes "FlowStep/FlowContainer" and older fields.

Gap:
- ROM schema (rom_version, step_type, properties, edges) is not defined or validated.

Needed:
- Formal ROM schema validation in runtime and designer.
- Migration of `raisin:Flow.workflow_data` to ROM format or versioned compatibility.

### 2) Message-First Human I/O
Current:
- `HumanTask` step exists in runtime, `raisin:InboxTask` exists as a node type.
- Messaging system is separate and not wired to flow resumption.

Gap:
- MessageStep (message-driven human steps) is not implemented.
- No standard resume trigger on message status transitions.

Needed:
- MessageStep handler that creates `raisin:Message` and waits on status changes.
- Trigger hook to resume FlowInstance when message status changes.
- Decide whether `raisin:InboxTask` is deprecated or mapped to messages.

### 3) AgentStep Enhancements (Beyond LangChain/Pydantic)
Current:
- AIContainer step exists with tool calls and wait/resume.
- No streaming, validation, or self-correction loop.

Gap:
- Streaming events, output validation, and model retry are missing.
- Dynamic tools and history processors are not supported.

Needed:
- Streaming output events and partial AIMessage updates.
- Output validation strategies (schema + validators).
- Self-correction retry policy for invalid outputs.
- Tool prepare functions and toolset routing.
- History processors and context management.

### 4) Flow Runtime Coverage
Current:
- Execution loop, OCC, and pause/resume are implemented.

Gap:
- No ROM event naming contract.
- Join/quorum semantics and message-based waits are partial.
- Subflow, loop, and custom steps are limited or missing.

Needed:
- ROM step coverage (message, subflow, loop, custom).
- Standardized event payloads and naming.
- Join policy (all vs quorum) and explicit wait reasons.

### 5) Designer Support
Current:
- Visual editor stores workflow_data but not ROM schema.

Gap:
- ROM step palette and property editors are not defined.
- No validation for ROM-specific fields.

Needed:
- Designer schema for ROM nodes and edges.
- UI for AgentStep, MessageStep, and advanced policies.
- Static validation of REL expressions and references.

### 6) Observability and Testing
Current:
- Step events exist, but no unified ROM event taxonomy.

Gap:
- No ROM event model, evaluation framework, or regression testing pipeline.

Needed:
- Standard ROM event stream (SSE/WebSocket).
- EvalDataset/EvalRun integration for agent testing.
- Observability integration (tracing, cost tracking).

### 7) Node Type Alignment
Current:
- `raisin:Flow` still documents the older workflow_data shape.

Gap:
- ROM is not reflected in node type documentation.

Needed:
- Update `raisin:Flow` description to reference ROM schema and versioning.

## Robust ROM Runtime: Minimum Path

Phase 1 (Core)
- Define ROM schema and validator.
- Implement MessageStep and message-based waits.
- Update `raisin:Flow.workflow_data` for ROM 1.0.
- Standardize ROM event emission.

Phase 2 (Agent Superiority)
- Streaming support (content deltas to AIMessage).
- Output validation and self-correction retry policies.
- Dynamic tools and history processors.

Phase 3 (Complexity and Scale)
- Subflow/loop/custom steps.
- Join policies (quorum), parallel metrics.
- Evaluation framework and regression suite.

Phase 4 (Designer and Ecosystem)
- ROM visual authoring and validation.
- Declarative schemas for step properties.
- Plugin system for custom ROM steps.

## Notes

- The existing `raisin-flow-runtime` architecture is strong; ROM gaps are primarily
  schema, message integration, and advanced agent features.
- Messaging already provides robust human I/O and status transitions; ROM should
  make it the default boundary rather than a separate task system.
