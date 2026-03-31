# raisin-flow-runtime

Stateful workflow execution runtime for RaisinDB.

## Overview

A workflow engine that enables complex, long-running processes with AI integration, human-in-the-loop steps, and saga-based compensation. Workflows can pause at async boundaries (function calls, AI operations, human tasks) and resume when results are available.

## Features

- **AI Agent Loops** - Multi-turn AI conversations with tool calls
- **Human-in-the-Loop** - Workflows that wait for human input/approval
- **Decision Trees** - Complex branching logic with conditions
- **Parallel Execution** - Fork/join patterns for concurrent steps
- **Loop Constructs** - For-each, while, and times-based iteration
- **Sub-Flows** - Reusable workflow composition
- **Saga Compensation** - Automatic rollback on failure
- **Hybrid Batching** - Sync steps batch together; async creates jobs

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    FlowInstance                              │
│  ┌─────────────────────────────────────────────────────┐    │
│  │  FlowDefinition (nodes, edges, triggers)            │    │
│  └─────────────────────────────────────────────────────┘    │
│  ┌─────────────────────────────────────────────────────┐    │
│  │  FlowContext (input, variables, step_outputs)       │    │
│  └─────────────────────────────────────────────────────┘    │
│  ┌─────────────────────────────────────────────────────┐    │
│  │  ExecutionState (current_node, status, stack)       │    │
│  └─────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────────┐
│                    FlowExecutor                              │
│                                                              │
│  1. Load current node from FlowDefinition                   │
│  2. Dispatch to appropriate StepHandler                     │
│  3. Handle StepResult:                                      │
│     - Continue → advance to next node                       │
│     - Wait → persist state, create job                      │
│     - Complete → mark flow complete                         │
│     - Error → trigger compensation                          │
└─────────────────────────────────────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────────┐
│                    Step Handlers                             │
│                                                              │
│  AiContainerHandler  - AI agent with tool calls             │
│  ChatStepHandler     - Single AI completion                 │
│  FunctionStepHandler - Function execution (Starlark/JS)     │
│  DecisionHandler     - Conditional branching                │
│  ParallelHandler     - Fork execution branches              │
│  LoopHandler         - Iteration constructs                 │
│  SubFlowHandler      - Child flow invocation                │
│  WaitHandler         - Scheduled delays                     │
│  HumanTaskHandler    - Human approval/input                 │
│  ErrorHandler        - Error handling and routing           │
└─────────────────────────────────────────────────────────────┘
```

## Step Types

| Step Type | Description | Status |
|-----------|-------------|--------|
| `AiContainer` | Multi-turn AI agent with tools | ✅ Complete |
| `Chat` | Single AI completion | ✅ Complete |
| `Function` | Function call (Starlark/JS) | ✅ Complete |
| `Decision` | Conditional branching | ✅ Complete |
| `Parallel` | Fork execution | ✅ Complete |
| `Loop` | For-each/while iteration | ⚠️ Partial |
| `SubFlow` | Child flow invocation | ⚠️ Partial |
| `Wait` | Scheduled delay | ⚠️ Partial |
| `HumanTask` | Human approval | ✅ Complete |
| `Error` | Error handling | ✅ Complete |

## Usage

### Creating a Flow Instance

```rust
use raisin_flow_runtime::{FlowInstance, FlowDefinition};

let instance = FlowInstance::new(
    "/flows/approval-workflow".to_string(),
    1, // version
    flow_definition_snapshot,
    serde_json::json!({ "document_id": "doc-123" }),
    "start".to_string(),
);
```

### Executing a Flow

```rust
use raisin_flow_runtime::{FlowExecutor, FlowCallbacks};

let executor = FlowExecutor::new();
let result = executor.execute(&mut instance, &callbacks).await?;

match result {
    StepResult::Continue(next) => { /* advance to next node */ }
    StepResult::Wait(info) => { /* persist and create job */ }
    StepResult::Complete(output) => { /* flow finished */ }
    StepResult::Error(err) => { /* handle error */ }
}
```

### Data Mapping

```rust
// Reference input data
"${input.user.email}"

// Reference step outputs
"${steps.validate.result.valid}"

// Reference variables
"${variables.approval_status}"

// Math expressions (via raisin-rel)
"${input.price * 1.2}"
```

## Modules

| Module | Description |
|--------|-------------|
| `compiler/` | Flow definition parsing and validation |
| `handlers/` | Step type implementations |
| `runtime/` | Execution engine, state management, compensation |
| `integration/` | Job system and trigger integration |
| `types/` | Core data structures |

### Runtime Components

| Component | Description |
|-----------|-------------|
| `executor.rs` | Main execution loop |
| `data_mapper.rs` | Expression evaluation with `${...}` syntax |
| `state_manager.rs` | Flow state persistence |
| `compensation.rs` | Saga rollback logic |
| `retry.rs` | Retry policies |
| `timeout.rs` | Step timeout handling |
| `subscription.rs` | Event subscription management |

## Execution Model

1. **Sync Batching** - Multiple sync steps execute without persistence
2. **Async Boundary** - When a step returns `Wait`, state is persisted
3. **Job Creation** - Async operations create jobs (function calls, AI, human tasks)
4. **Resume** - When job completes, flow resumes from saved state
5. **OCC** - Optimistic concurrency control via revision numbers

## Compensation (Saga Pattern)

```rust
// Register compensation when step succeeds
context.push_compensation(CompensationEntry {
    step_id: "create_order".into(),
    compensation_type: CompensationType::Function,
    compensation_data: json!({ "function": "cancel_order" }),
});

// On failure, compensations execute in LIFO order
executor.rollback_flow(&mut instance, &callbacks).await?;
```

## Crate Usage

Used by:
- `raisin-rocksdb` - Flow callbacks and instance execution
- `raisin-transport-http` - Flow API endpoints and triggers

## Roadmap

See [ANALYSIS_AND_PROPOSAL.md](./ANALYSIS_AND_PROPOSAL.md) for detailed gap analysis and implementation roadmap including:
- Full REL expression integration for loop conditions
- Robust parallel join implementation
- Async compensation support
- Execution quotas and limits

## License

BSL-1.1 - See [LICENSE](../../LICENSE) for details.
