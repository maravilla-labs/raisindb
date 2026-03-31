# RaisinDB Flow Runtime Concept

## Vision
Make RaisinDB a better alternative to LangChain/LangFlow by integrating stateful workflow execution with the multimodal database. Flows are first-class citizens that can orchestrate AI agents, functions, and human tasks.

## Problem Statement
The current job system is good for simple triggers and function execution, but not suitable for:
- AI agent loops with multiple tool calls
- Human-in-the-loop workflows
- Complex decision trees with branching
- Long-running workflows that need to pause/resume
- Interactive chat-like experiences

## Final Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Execution Model | Hybrid Batching | Efficient sync execution, jobs only for async ops |
| AI Agent Loops | Configurable via AI Container | Side panel config, auto/explicit/hybrid modes |
| Human Tasks | User Inbox (MessageFolder) | Reuse existing structure |
| Flow Storage | Dedicated `flows` workspace | Flows are orchestrators, conversations are children |
| Architecture | New `raisin-flow-runtime` crate | Clean separation, uses existing crates |
| Migration | Replace ai-tools triggers | Single unified approach |
| First Target | AI agent chat | Validates core loop and AI container |
| **Parallel Execution** | **Phase 1** | **Fundamental feature, not advanced** |
| **Rollback Strategy** | **Saga compensation (v1)** | **Simpler than branching, battle-tested** |
| **Git-like branching** | **Future (v2+)** | **Add when needed, avoid premature complexity** |

---

## Current Architecture Analysis

### What Works
1. **Job System**: Reliable for fire-and-forget tasks, uses unified JobRegistry pattern
2. **Trigger Matching**: Sophisticated pattern matching with filters and priorities
3. **AI-Tools Plugin**: Proves the concept works with trigger-based event loop
4. **Flow Designer**: Rich visual editor with conditions, containers, parallel execution

### Gaps
1. **No stateful flow execution** - can't pause/resume
2. **No visual flow runtime** - workflow_data not connected to execution
3. **AI loops are hacky** - trigger chains with idempotency checks
4. **No human-in-the-loop** - can't wait for user input
5. **No branching** - can't take different paths based on results

---

## Proposed Concept: Raisin Flow Runtime

### Core Idea
A **stateful flow execution engine** that:
1. Interprets visual workflow definitions from raisin-flow-designer
2. Maintains execution state as nodes (like AIConversation pattern)
3. Can pause awaiting external events (tool results, human input, scheduled time)
4. Supports AI agent loops natively
5. Separates from job system but can trigger jobs for function execution

### Key Concepts

#### 1. Flow Instance (raisin:FlowInstance)
A running instance of a flow definition, stored as a node:
- References parent flow definition
- Contains execution state (current step, variables, context)
- Children are execution records (steps taken, decisions made)
- Can be paused, resumed, cancelled
- Persistent - survives restarts

#### 2. Execution State Machine
States a flow instance can be in:
- `pending` - Created but not started
- `running` - Actively executing
- `waiting_for_tool` - Paused awaiting tool/function result
- `waiting_for_human` - Paused awaiting human interaction
- `waiting_for_event` - Paused awaiting external trigger
- `completed` - Successfully finished
- `failed` - Error occurred
- `cancelled` - Manually stopped

#### 3. Step Types (Extended from Flow Designer)
- **Function Step**: Execute a raisin function (async, via job system)
- **Agent Step**: Send to AI agent, handle response and tool calls
- **Human Task**: Create task in user inbox, await response
- **Decision**: Evaluate raisin-rel condition, branch accordingly
- **Parallel Gateway**: Fork execution into parallel branches
- **Join Gateway**: Synchronize parallel branches
- **Wait Step**: Pause for time, event, or condition
- **Sub-Flow**: Execute another flow as a step

#### 4. AI Agent Loop Integration
When an agent step executes:
1. Flow instance enters `waiting_for_tool` if tool calls needed
2. Tool calls executed as child flow instances or jobs
3. Results collected and synchronized (like current on-tool-result handler)
4. AI called again with results
5. Loop continues until no more tool calls
6. Agent response becomes step output, flow continues

#### 5. Human-in-the-Loop
When a human task step executes:
1. Creates task node in target user's inbox (raisin:Task or raisin:Message)
2. Flow instance enters `waiting_for_human`
3. Task node references flow instance
4. When user completes task → trigger fires
5. Flow instance resumes with user's response

#### 6. Event-Driven Resumption
Flow instances can be resumed by:
- Node events (tool result created, human task completed)
- Scheduled triggers (cron-based wake-up)
- HTTP webhooks (external system callback)
- Internal signals (another flow completing)

---

## Architecture Options

### Option A: Pure Node-Event Architecture (Current Pattern Extended)
Continue using triggers but add flow state management:
- Flow instance as node with state property
- Each step execution creates child nodes
- Triggers match on state changes to continue flow
- Pros: Uses existing infrastructure
- Cons: Complex trigger chains, hard to debug, poor visibility

### Option B: Dedicated Flow Runtime Service
Separate stateful service that manages flow execution:
- Long-running process with in-memory state
- Persists state to nodes periodically
- Direct function/agent calls without job indirection
- Pros: Better performance, cleaner execution model
- Cons: New infrastructure, harder to scale, memory concerns

### Option C: Hybrid (Recommended)
Flow runtime as a job handler type with special capabilities:
- `FlowExecution` job carries flow instance ID
- Handler loads state from nodes, executes until pause point
- Saves state back, schedules continuation if needed
- Uses existing job infrastructure for durability
- Direct callbacks for function execution (no nested jobs)

---

## Data Model

### New Node Types

#### raisin:FlowInstance
Execution instance of a flow
```yaml
properties:
  - flow_ref: Reference to raisin:Flow definition
  - status: pending | running | waiting_* | completed | failed | cancelled
  - current_step_id: Current position in flow
  - variables: Object - flow-scoped variables
  - input: Object - initial input to flow
  - output: Object - final output (when completed)
  - error: String - error message (when failed)
  - started_at: DateTime
  - completed_at: DateTime
  - wait_info: Object - what we're waiting for (tool, human, event)
allowed_children:
  - raisin:FlowStepExecution
  - raisin:FlowEvent
```

#### raisin:FlowStepExecution
Record of a step being executed
```yaml
properties:
  - step_id: Which step in the flow definition
  - status: pending | running | completed | failed | skipped
  - input: Object - input to this step
  - output: Object - output from this step
  - error: String - error if failed
  - started_at: DateTime
  - completed_at: DateTime
  - retry_count: Integer
allowed_children:
  - raisin:FlowStepExecution (for nested/sub-flow steps)
```

#### raisin:FlowEvent
Event that occurred during flow execution
```yaml
properties:
  - event_type: tool_call | tool_result | human_response | timeout | error
  - payload: Object
  - timestamp: DateTime
```

#### raisin:Task (or extend existing)
Human task awaiting response
```yaml
properties:
  - title: String
  - description: String
  - flow_instance_ref: Reference to FlowInstance
  - step_id: Which step created this
  - status: pending | completed | cancelled
  - due_date: DateTime
  - priority: Integer
  - response: Object - user's response
```

---

## Execution Flow Example

### AI Chat Workflow
```
User sends message
    ↓
Trigger: on node created (AIMessage, role=user)
    ↓
Creates FlowInstance for "agent-conversation-flow"
    ↓
Flow Runtime executes:
  1. [Agent Step] Call AI with history
     - AI returns with tool_calls
  2. [Parallel Gateway] Fork for each tool call
     - [Function Step] Execute tool 1
     - [Function Step] Execute tool 2
  3. [Join Gateway] Wait for all tools
  4. [Decision] Has tool results?
     - Yes → goto step 1 (loop)
     - No → continue
  5. [Decision] Needs human approval?
     - Yes → [Human Task] Create approval request
     - No → continue
  6. Create assistant message with response
```

### Approval Workflow with Human-in-Loop
```
Document submitted for review
    ↓
Creates FlowInstance for "document-approval-flow"
    ↓
Flow Runtime executes:
  1. [Function Step] Validate document format
  2. [Agent Step] AI reviews content, suggests category
  3. [Human Task] Manager approves/rejects
     - Flow pauses, task in manager's inbox
     - Manager responds
  4. [Decision] Approved?
     - Yes → [Function Step] Publish document
     - No → [Function Step] Notify submitter
  5. Complete
```

---

## Implementation Phases

### Phase 1: Foundation
- [ ] Define node types (FlowInstance, FlowStepExecution, etc.)
- [ ] Create FlowRuntime handler/service
- [ ] Basic state machine execution
- [ ] Step execution for functions (via callbacks)

### Phase 2: AI Agent Integration
- [ ] Agent step type with tool call handling
- [ ] Tool execution with parallel/join pattern
- [ ] Loop detection and handling
- [ ] Conversation history building

### Phase 3: Human-in-Loop
- [ ] Task node type and inbox integration
- [ ] Trigger for task completion
- [ ] Flow resumption from human response
- [ ] UI for task management

### Phase 4: Flow Designer Integration
- [ ] Connect workflow_data to runtime
- [ ] Visual debugging (show flow instance state)
- [ ] Step-through execution mode
- [ ] Error visualization

### Phase 5: Advanced Features
- [ ] Sub-flows
- [ ] Error compensation/rollback
- [ ] Timeouts and SLAs
- [ ] Flow versioning (running old definition)

---

## Design Decisions (Confirmed)

1. **Execution Model: Hybrid Batching**
   - Runtime executes synchronous steps continuously
   - Creates new jobs only for async operations (function calls, AI, human tasks)
   - Efficient execution with granular async handling

2. **AI Agent Loops: Configurable via AI Container**
   - Agent step is a pointer to `raisin:Agent` node type
   - AI Container (existing in flow designer) handles configuration
   - Side panel in flow designer configures: agent ref, tool behavior, max iterations, thinking mode
   - Can do internal loop OR expose tool calls as explicit sub-flow

3. **Human Tasks: User Inbox (MessageFolder)**
   - Use existing inbox structure
   - Tasks appear as special messages user responds to
   - Flow waits for response in inbox

4. **Migration: Replace with Flows**
   - ai-tools plugin becomes template flows
   - Current trigger-based approach is deprecated
   - Cleaner architecture, one system instead of two

---

## AI Container Concept

The existing `AI Sequence` container in flow designer becomes the primary way to configure AI agent behavior:

```
┌─────────────────────────────────────────────┐
│  AI Container: "Customer Support Agent"     │
│  ┌─────────────────────────────────────┐   │
│  │ Agent: /agents/support-agent        │   │
│  │ Tool Loop: Internal (max 5 iter)    │   │
│  │ Thinking: Enabled                   │   │
│  │ On Tool Error: Continue             │   │
│  └─────────────────────────────────────┘   │
│                                             │
│  Children (optional explicit tool steps):   │
│  ├── [Tool Step] lookup-customer           │
│  ├── [Tool Step] check-orders              │
│  └── [Decision] needs-escalation?          │
│       ├── Yes → [Human Task] manager       │
│       └── No → continue                    │
└─────────────────────────────────────────────┘
```

### AI Container Modes:
1. **Auto mode**: Agent handles tool calls internally, container completes when agent is done
2. **Explicit mode**: Tool calls appear as child steps, giving visual control over the loop
3. **Hybrid**: Some tools internal, specific tools as explicit steps for special handling

### AI Container Properties (Side Panel):
- `agent_ref`: Reference to raisin:Agent node
- `tool_mode`: "auto" | "explicit" | "hybrid"
- `explicit_tools`: List of tools to expose as steps (when hybrid)
- `max_iterations`: Limit on tool call loops (default 10)
- `thinking_enabled`: Show/store AI reasoning
- `on_error`: "stop" | "continue" | "retry"
- `timeout_ms`: Max time for entire container execution

---

## Critical Technical Requirements

### A. Concurrency Control (The "Double Click" Problem)

**Scenario**: Flow is `waiting_for_human`. Two managers click "Approve" at the exact same millisecond.

**Risk**: Both triggers fire, both load FlowInstance, both advance state → next step executes twice (e.g., sending money twice).

**Solution**: Optimistic Concurrency Control (OCC) on FlowInstance

```rust
// When loading instance, note the _version
let instance = load_instance(id).await?;
let expected_version = instance.version;

// ... execute step ...

// When saving, assert version hasn't changed
match save_instance_with_version_check(&instance, expected_version).await {
    Ok(_) => { /* success */ },
    Err(VersionConflict) => {
        // Another process already advanced the flow
        // Either retry (reload and check if we still need to act)
        // Or fail gracefully
    }
}
```

### B. Flow Definition Versioning (In-Flight Flows)

**Scenario**: Flow definition updated to add a step. Old flow instance is paused at Step 2. When it resumes, which definition?

**Solution**: Snapshot flow definition at instance creation

```yaml
raisin:FlowInstance:
  properties:
    - name: flow_definition_snapshot
      type: Object
      description: Complete workflow_data copied at instance creation (immutable)
    - name: flow_ref
      type: Reference
      description: Reference to original raisin:Flow (for UI linking)
    - name: flow_version
      type: Integer
      description: Version of flow definition at creation time
```

The instance always uses `flow_definition_snapshot`, not the current head of the flow.

### C. Saga Compensation Pattern (v1 Rollback Strategy)

**Why not git-like branching for v1**:
- Adds complexity (branch lifecycle, merge conflicts, cleanup)
- Most workflows don't need it
- Saga compensation is battle-tested in distributed systems
- Can add branching later as advanced feature (v2+)

**Saga Pattern**:
Each step can optionally define a compensation function. On failure, execute compensations in reverse order.

```
Flow Execution:
    Step 1: reserve_inventory()     → push compensation: release_inventory()
    Step 2: charge_payment()        → push compensation: refund_payment()
    Step 3: send_confirmation()     → FAILS!

Rollback:
    Execute: refund_payment()       ← pop from stack
    Execute: release_inventory()    ← pop from stack
    Flow marked as: rolled_back
```

**Compensation Stack on FlowInstance**:
```yaml
raisin:FlowInstance:
  properties:
    - name: compensation_stack
      type: Array
      description: Stack of completed steps with undo info for rollback
      items:
        type: Object
        properties:
          step_id: String
          completed_at: DateTime
          compensation_fn: String       # Function to call to undo
          compensation_input: Object    # Data needed for undo
          compensation_status: String   # pending | executed | failed
```

**Step Definition with Compensation**:
```yaml
raisin:FlowStep:
  properties:
    - name: compensation_ref
      type: Reference
      description: Function to call to undo this step's side effects
    - name: compensation_input_mapping
      type: Object
      description: How to map step output to compensation input
```

**Implementation**:
```rust
async fn execute_step_with_compensation(
    step: &FlowStep,
    instance: &mut FlowInstance,
) -> Result<StepResult> {
    let result = execute_step(step, instance).await?;

    // If step has compensation, push to stack
    if let Some(comp_fn) = &step.compensation_ref {
        instance.compensation_stack.push(CompensationEntry {
            step_id: step.id.clone(),
            completed_at: Utc::now(),
            compensation_fn: comp_fn.clone(),
            compensation_input: map_compensation_input(&step, &result),
            compensation_status: "pending".to_string(),
        });
    }

    Ok(result)
}

async fn rollback_flow(instance: &mut FlowInstance) -> Result<()> {
    while let Some(mut entry) = instance.compensation_stack.pop() {
        match execute_compensation(&entry).await {
            Ok(_) => entry.compensation_status = "executed".to_string(),
            Err(e) => {
                entry.compensation_status = "failed".to_string();
                // Log error but continue with other compensations
                tracing::error!("Compensation failed for {}: {}", entry.step_id, e);
            }
        }
        // Save progress in case rollback is interrupted
        save_instance(instance).await?;
    }
    instance.status = FlowStatus::RolledBack;
    Ok(())
}
```

**When compensation is needed**:
- External API calls (payment, email, SMS)
- Third-party service integrations
- Any side effect outside RaisinDB

**When compensation is NOT needed**:
- Pure RaisinDB data operations (can use versioning/restore if needed)
- Read-only steps
- AI inference (no side effects)

### D. Future: Git-Like Branching (v2+)

For advanced use cases, add git-like branching later:
- Entire flow in isolated branch
- Per-step branching for parallel agents
- Merge conflict resolution UI
- Branch preview before merge

This is **not in scope for v1** to keep complexity manageable.

### E. Observability Requirements

**Metrics to track**:
- Flow execution duration (total and per-step)
- Flow success/failure/rollback rates
- Retry counts and failure reasons
- Human task response times
- AI container iteration counts
- Compensation execution success rate

**Dashboard views**:
- Active flows (running, waiting)
- Flow history with filtering
- Step-level drill-down
- Error rates and trends
- Human task queue depth

**Inbox UX for Human Tasks**:
- Clear indication task is part of a flow
- Show flow name and context
- Display what user is approving/rejecting
- Show deadline/SLA if configured
- Link to full flow visualization

### F. Testing Requirements (Corner Cases)

**Must test in Phase 1**:
- Double-click problem (OCC prevents duplicate execution)
- Cancelled flow cleans up pending human tasks
- Long-running flow timeout handling
- Retry exhaustion and proper failure state
- Compensation execution on failure
- Compensation failure handling (continue vs stop)

**Must test in Phase 2 (AI)**:
- AI container max iterations reached
- Tool call timeout
- Parallel tool execution synchronization
- LLM provider failure and retry

### G. Event Subscription Index (O(1) Lookup)

**Problem**: When tool result arrives, how to find the waiting FlowInstance efficiently?

**Solution**: Formalize `wait_info` with subscription ID and maintain an index

```yaml
raisin:FlowInstance:
  properties:
    - name: wait_info
      type: Object
      description: Structured subscription for resumption
      properties:
        subscription_id: String     # Unique ID for this wait
        wait_type: String           # tool_call | human_task | scheduled | event
        target_path: String         # Path being watched (e.g., inbox task path)
        expected_event: String      # Event type to match
        timeout_at: DateTime        # When to auto-fail if no response
```

Runtime maintains an in-memory or RocksDB index: `subscription_id → instance_id`

### E. Cancellation Propagation

When flow is cancelled:
1. Find all pending child tasks (InboxTask, sub-flows, etc.)
2. Mark them as `cancelled`
3. User sees cancelled task, doesn't approve a dead workflow

```rust
async fn cancel_flow(instance_id: &str) -> Result<()> {
    let instance = load_instance(instance_id).await?;

    // Cancel any pending inbox tasks
    if let Some(wait_info) = &instance.wait_info {
        if wait_info.wait_type == "human_task" {
            cancel_inbox_task(&wait_info.target_path).await?;
        }
    }

    // Cancel any sub-flows
    for child in get_child_flows(&instance).await? {
        cancel_flow(&child.id).await?;
    }

    instance.status = FlowStatus::Cancelled;
    save_instance(&instance).await?;
}
```

---

## Refined Data Model

### raisin:FlowInstance
```yaml
name: raisin:FlowInstance
description: Running instance of a flow definition
icon: play-circle
version: 1
properties:
  # Core identification
  - name: flow_ref
    type: Reference
    description: Reference to original raisin:Flow (for UI linking)
  - name: flow_version
    type: Integer
    description: Version of flow definition at creation time
  - name: flow_definition_snapshot
    type: Object
    description: Complete workflow_data copied at instance creation (immutable)

  # Execution state
  - name: status
    type: String
    enum: [pending, running, waiting, completed, failed, cancelled]
  - name: current_node_id
    type: String
    description: Current position in flow

  # Wait management (formalized)
  - name: wait_info
    type: Object
    description: Structured subscription for resumption
    properties:
      subscription_id: String
      wait_type: String  # tool_call | human_task | scheduled | event
      target_path: String
      expected_event: String
      timeout_at: DateTime

  # Context and data
  - name: variables
    type: Object
    description: Flow-scoped variables (mutable during execution)
  - name: input
    type: Object
    description: Initial input to flow
  - name: output
    type: Object
    description: Final output (when completed)

  # Saga/rollback support
  - name: compensation_stack
    type: Array
    description: Stack of completed steps with undo info for rollback
    items:
      type: Object

  # Error handling
  - name: error
    type: String
  - name: retry_count
    type: Integer
    default: 0

  # Timestamps
  - name: started_at
    type: DateTime
  - name: completed_at
    type: DateTime

  # Hierarchy
  - name: parent_instance_ref
    type: Reference
    description: Parent flow instance (for sub-flows)

  # Metrics/observability
  - name: metrics
    type: Object
    description: Execution metrics for monitoring
    properties:
      total_duration_ms: Integer
      step_count: Integer
      retry_count: Integer
      compensation_count: Integer

allowed_children:
  - raisin:FlowStepExecution
  - raisin:AIConversation  # AI container creates conversations as children

versionable: true  # For OCC
auditable: true
```

### raisin:FlowStepExecution
```yaml
name: raisin:FlowStepExecution
description: Record of a step execution
properties:
  - name: node_id
    type: String
    description: Which node in flow definition
  - name: status
    type: String
    enum: [pending, running, completed, failed, skipped]
  - name: input
    type: Object
  - name: output
    type: Object
  - name: error
    type: String
  - name: started_at
    type: DateTime
  - name: completed_at
    type: DateTime
  - name: iteration
    type: Integer
    description: For loops, which iteration
```

### Extended raisin:FlowContainer (for AI)
```yaml
# Additional properties when container_type = "ai_sequence"
properties:
  - name: agent_ref
    type: Reference
    description: Reference to raisin:Agent
  - name: tool_mode
    type: String
    enum: [auto, explicit, hybrid]
    default: auto
  - name: explicit_tools
    type: Array
    items:
      type: String
    description: Tools to expose as steps (hybrid mode)
  - name: max_iterations
    type: Integer
    default: 10
  - name: thinking_enabled
    type: Boolean
    default: false
  - name: conversation_ref
    type: Reference
    description: Existing conversation to continue (optional)
```

---

## Runtime Architecture

### Flow Runtime Service
```
┌─────────────────────────────────────────────────────────┐
│                    Flow Runtime                          │
├─────────────────────────────────────────────────────────┤
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐     │
│  │   Loader    │  │  Executor   │  │  State Mgr  │     │
│  │ (flow def)  │  │ (step exec) │  │ (persist)   │     │
│  └─────────────┘  └─────────────┘  └─────────────┘     │
│         │                │                │             │
│         └────────────────┼────────────────┘             │
│                          │                              │
│  ┌─────────────────────────────────────────────────┐   │
│  │              Step Handlers                       │   │
│  │  ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌───────┐ │   │
│  │  │Function │ │   AI    │ │ Human   │ │Decision│ │   │
│  │  │ Handler │ │Container│ │  Task   │ │Handler │ │   │
│  │  └─────────┘ └─────────┘ └─────────┘ └───────┘ │   │
│  └─────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────┘
         │                    │                    │
         ▼                    ▼                    ▼
    ┌─────────┐         ┌─────────┐         ┌─────────┐
    │Job Queue│         │AI Provider│        │User Inbox│
    └─────────┘         └─────────┘         └─────────┘
```

### Execution Loop (Hybrid Batching with OCC and Error Handling)
```rust
async fn execute_flow(instance_id: &str) -> Result<()> {
    // 1. Load Instance with version for OCC
    let mut instance = load_instance(instance_id).await?;
    let expected_version = instance.version;

    // 2. Skip if already completed (idempotency check)
    if instance.status == FlowStatus::Completed || instance.status == FlowStatus::Cancelled {
        return Ok(());
    }

    loop {
        let current_step = get_step_from_snapshot(&instance, &instance.current_node_id)?;

        // 3. Execute step based on type
        let result = match current_step.step_type {
            StepType::Decision => {
                decision_handler::execute(&current_step, &instance.variables).await
            },
            StepType::Function => {
                function_handler::execute(&current_step, &instance).await
            },
            StepType::AIContainer => {
                ai_container_handler::execute(&current_step, &mut instance).await
            },
            StepType::HumanTask => {
                human_task_handler::execute(&current_step, &mut instance).await
            },
            StepType::End => {
                StepResult::Complete { output: instance.variables.clone() }
            },
            _ => StepResult::Continue {
                next_node_id: current_step.next_node.clone(),
                output: Value::Null
            }
        };

        match result {
            // 4. Sync Success -> Move to next step immediately (Hot Path)
            StepResult::Continue { next_node_id, output } => {
                instance.variables.merge(&output);
                instance.current_node_id = next_node_id;
                // OPTIMIZATION: Don't write to DB yet if next step is also sync
                // Only persist at async boundaries
            },

            // 5. Async Boundary -> Persist and Exit
            StepResult::Wait { reason, metadata } => {
                instance.status = FlowStatus::Waiting;
                instance.wait_info = Some(WaitInfo {
                    subscription_id: generate_subscription_id(),
                    wait_type: reason.to_string(),
                    target_path: metadata.get("target_path").cloned(),
                    expected_event: metadata.get("expected_event").cloned(),
                    timeout_at: calculate_timeout(&current_step),
                });

                // HARD COMMIT with OCC check
                match save_instance_with_version(&instance, expected_version).await {
                    Ok(_) => {
                        // Register subscription for O(1) lookup
                        register_wait_subscription(&instance.wait_info.as_ref().unwrap()).await?;
                        return Ok(());
                    },
                    Err(VersionConflict) => {
                        // Another process advanced the flow - reload and check
                        tracing::warn!("Version conflict on flow {}, reloading", instance_id);
                        return execute_flow(instance_id).await; // Retry with fresh state
                    }
                }
            },

            // 6. Flow completed
            StepResult::Complete { output } => {
                instance.status = FlowStatus::Completed;
                instance.output = Some(output);
                instance.completed_at = Some(Utc::now());
                save_instance_with_version(&instance, expected_version).await?;
                return Ok(());
            },

            // 7. Error handling with retry
            StepResult::Error { error } => {
                let step_config = get_step_error_config(&current_step);

                if instance.retry_count < step_config.max_retries {
                    instance.retry_count += 1;
                    let backoff = calculate_backoff(instance.retry_count); // 10s, 30s, 60s
                    instance.status = FlowStatus::Waiting;
                    instance.wait_info = Some(WaitInfo {
                        subscription_id: generate_subscription_id(),
                        wait_type: "retry".to_string(),
                        timeout_at: Some(Utc::now() + backoff),
                        ..Default::default()
                    });
                    save_instance_with_version(&instance, expected_version).await?;
                    schedule_retry(&instance).await?;
                    return Ok(());
                } else {
                    // Max retries exceeded - fail the flow
                    instance.status = FlowStatus::Failed;
                    instance.error = Some(error.to_string());
                    instance.completed_at = Some(Utc::now());
                    save_instance_with_version(&instance, expected_version).await?;
                    return Err(error);
                }
            }
        }
    }
}
```

### AI Container Execution
```rust
async fn execute_ai_container(
    instance: &mut FlowInstance,
    container: &FlowNode
) -> Result<()> {
    let agent = load_agent(&container.agent_ref)?;
    let mut iteration = 0;

    loop {
        iteration += 1;
        if iteration > container.max_iterations {
            return Err(FlowError::MaxIterationsExceeded);
        }

        // Build conversation history from instance children
        let history = build_conversation_history(instance)?;

        // Call AI
        let response = call_ai(&agent, &history).await?;

        // Create AIMessage child
        create_ai_message(instance, &response).await?;

        // Handle tool calls
        if response.tool_calls.is_empty() {
            // Done - AI has no more tool calls
            instance.current_node_id = container.next_node;
            return continue_flow(instance).await;
        }

        match container.tool_mode {
            ToolMode::Auto => {
                // Execute tools internally
                let results = execute_tools_parallel(&response.tool_calls).await?;
                // Loop continues with results
            }
            ToolMode::Explicit => {
                // Pause - tool calls become child steps
                for tool_call in response.tool_calls {
                    create_tool_step(instance, &tool_call).await?;
                }
                instance.status = FlowStatus::Waiting;
                instance.wait_reason = WaitReason::ToolCalls;
                return Ok(());
            }
            ToolMode::Hybrid => {
                // Some auto, some explicit
                let (auto_tools, explicit_tools) = partition_tools(
                    &response.tool_calls,
                    &container.explicit_tools
                );
                execute_tools_parallel(&auto_tools).await?;
                if !explicit_tools.is_empty() {
                    for tool in explicit_tools {
                        create_tool_step(instance, &tool).await?;
                    }
                    instance.status = FlowStatus::Waiting;
                    return Ok(());
                }
            }
        }
    }
}
```

---

## User Inbox Integration

When a flow needs human input:

```
┌─────────────────────────────────────────┐
│ User: alice@company.com                 │
│ ├── profile/                            │
│ ├── inbox/                              │
│ │   └── task-approve-expense-123        │  ← Flow creates this
│ │       properties:                     │
│ │         task_type: approval           │
│ │         title: "Approve expense $500" │
│ │         flow_instance_ref: /flows/... │
│ │         step_id: "approval-step"      │
│ │         options: [approve, reject]    │
│ │         due_date: 2024-01-15          │
│ │         status: pending               │
│ └── outbox/                             │
└─────────────────────────────────────────┘
```

### Inbox Task Node Type
```yaml
name: raisin:InboxTask
description: Task awaiting user action
properties:
  - name: task_type
    type: String
    enum: [approval, input, review, action]
  - name: title
    type: String
    required: true
  - name: description
    type: String
  - name: flow_instance_ref
    type: Reference
    description: Flow instance waiting for response
  - name: step_id
    type: String
  - name: options
    type: Array
    items:
      type: Object
    description: Available response options
  - name: input_schema
    type: Object
    description: JSON schema for required input
  - name: due_date
    type: DateTime
  - name: priority
    type: Integer
  - name: status
    type: String
    enum: [pending, completed, expired, cancelled]
  - name: response
    type: Object
    description: User's response
  - name: responded_at
    type: DateTime
```

### Trigger for Task Completion
```yaml
name: on-inbox-task-completed
trigger_type: node_event
config:
  event_kinds: [Updated]
filters:
  node_types: [raisin:InboxTask]
  property_filters:
    - property: status
      value: completed
function_flow:
  # Resume the waiting flow instance
  steps:
    - id: resume-flow
      function_ref: /lib/raisin/flow-resume
```

---

## Migration from ai-tools

The ai-tools plugin transforms into template flows:

### Before (Trigger-based)
```yaml
# on-user-message trigger → agent-handler function
# on-tool-call trigger → tool-executor function
# on-tool-result trigger → agent-continue-handler function
```

### After (Flow-based)
```yaml
# Template flow: "agent-conversation-flow"
workflow_data:
  nodes:
    - id: start
      type: start
    - id: ai-container
      type: ai_sequence
      properties:
        tool_mode: auto
        max_iterations: 10
    - id: end
      type: end
  edges:
    - from: start, to: ai-container
    - from: ai-container, to: end
```

### Migration Path
1. ai-tools plugin provides default flow templates
2. Existing AIConversation nodes work with new runtime
3. New trigger watches for AIMessage creation
4. Trigger starts flow instance instead of calling handler directly
5. Flow runtime handles the conversation loop

---

## Flows Workspace Structure

```
flows/                              # Dedicated workspace for flow instances
├── instances/                      # Running/completed flow instances
│   └── schedule-meeting-abc123/    # FlowInstance node
│       ├── step-1-validate/        # FlowStepExecution
│       ├── step-2-ai-container/    # FlowStepExecution
│       │   └── conversation/       # AIConversation (created by AI container)
│       │       ├── msg-user-1/     # AIMessage
│       │       ├── msg-assistant-1/
│       │       └── ...
│       ├── step-3-human-approval/  # FlowStepExecution
│       │   └── task-ref → /users/alice/inbox/task-xyz
│       └── step-4-complete/
├── templates/                      # Reusable flow templates
│   └── agent-conversation/         # Default AI chat flow
└── archive/                        # Completed/cancelled instances (optional)
```

---

## New Crate: raisin-flow-runtime

### Crate Structure
```
crates/raisin-flow-runtime/
├── Cargo.toml
├── src/
│   ├── lib.rs                 # Public API
│   ├── types/
│   │   ├── mod.rs
│   │   ├── flow_instance.rs   # FlowInstance, FlowStatus, WaitReason
│   │   ├── step_execution.rs  # FlowStepExecution
│   │   ├── flow_definition.rs # Parse workflow_data from raisin:Flow
│   │   └── context.rs         # FlowContext (variables, input, etc.)
│   ├── runtime/
│   │   ├── mod.rs
│   │   ├── executor.rs        # Main execution loop (hybrid batching)
│   │   ├── state_manager.rs   # Load/save instance state to nodes
│   │   └── resume.rs          # Resume from waiting state
│   ├── handlers/
│   │   ├── mod.rs
│   │   ├── decision.rs        # Evaluate raisin-rel conditions
│   │   ├── function_step.rs   # Execute function via job queue
│   │   ├── ai_container.rs    # AI agent loop handling
│   │   ├── human_task.rs      # Create inbox task, handle response
│   │   ├── parallel.rs        # Fork/join gateway handling
│   │   └── subflow.rs         # Execute nested flow
│   └── integration/
│       ├── mod.rs
│       ├── job_handler.rs     # FlowExecution job handler
│       └── triggers.rs        # Flow start/resume triggers
```

### Dependencies
```toml
[dependencies]
raisin-storage = { path = "../raisin-storage" }  # Job types, storage traits
raisin-rel = { path = "../raisin-rel" }          # Condition evaluation
raisin-models = { path = "../raisin-models" }    # Node types, references
raisin-ai = { path = "../raisin-ai" }            # AI provider calls
serde = { version = "1", features = ["derive"] }
tokio = { version = "1", features = ["full"] }
tracing = "0.1"
```

### Key Traits
```rust
/// Step handler trait - each step type implements this
#[async_trait]
pub trait StepHandler: Send + Sync {
    /// Execute the step, returning next action
    async fn execute(
        &self,
        step: &FlowNode,
        context: &mut FlowContext,
        callbacks: &FlowCallbacks,
    ) -> Result<StepResult, FlowError>;
}

/// Result of step execution
pub enum StepResult {
    /// Continue to next step
    Continue { next_node_id: String, output: Value },
    /// Pause execution, waiting for external event
    Wait { reason: WaitReason, metadata: Value },
    /// Step completed, check for more steps
    Complete { output: Value },
    /// Flow failed
    Error { error: FlowError },
}

/// Callbacks provided by transport layer
pub struct FlowCallbacks {
    pub create_node: Box<dyn Fn(CreateNodeRequest) -> BoxFuture<Result<Node>>>,
    pub update_node: Box<dyn Fn(UpdateNodeRequest) -> BoxFuture<Result<Node>>>,
    pub get_node: Box<dyn Fn(GetNodeRequest) -> BoxFuture<Result<Option<Node>>>>,
    pub call_ai: Box<dyn Fn(AiRequest) -> BoxFuture<Result<AiResponse>>>,
    pub queue_job: Box<dyn Fn(JobRequest) -> BoxFuture<Result<JobId>>>,
}
```

---

## Implementation Plan

### Phase 1: Foundation (Core Runtime with Parallel & Saga Compensation)
**Goal**: Full flow execution with OCC, parallel containers, and saga-based rollback

**Files to create**:
- `crates/raisin-flow-runtime/Cargo.toml`
- `crates/raisin-flow-runtime/src/lib.rs`
- `crates/raisin-flow-runtime/src/types/mod.rs`
- `crates/raisin-flow-runtime/src/types/flow_instance.rs` - FlowInstance, FlowStatus, WaitInfo
- `crates/raisin-flow-runtime/src/types/step_execution.rs` - FlowStepExecution
- `crates/raisin-flow-runtime/src/types/flow_definition.rs` - Parse workflow_data
- `crates/raisin-flow-runtime/src/types/context.rs` - FlowContext (variables)
- `crates/raisin-flow-runtime/src/types/compensation.rs` - CompensationEntry, CompensationStack
- `crates/raisin-flow-runtime/src/runtime/mod.rs`
- `crates/raisin-flow-runtime/src/runtime/executor.rs` - Main loop with OCC
- `crates/raisin-flow-runtime/src/runtime/state_manager.rs` - Load/save with version check
- `crates/raisin-flow-runtime/src/runtime/subscription.rs` - Wait subscription index
- `crates/raisin-flow-runtime/src/runtime/compensation.rs` - Saga rollback execution
- `crates/raisin-flow-runtime/src/handlers/mod.rs`
- `crates/raisin-flow-runtime/src/handlers/decision.rs` - raisin-rel evaluation
- `crates/raisin-flow-runtime/src/handlers/function_step.rs` - Queue function job
- `crates/raisin-flow-runtime/src/handlers/parallel.rs` - Parallel container (fork/join)

**Files to modify**:
- `Cargo.toml` (workspace) - add new crate
- `crates/raisin-core/global_nodetypes/raisin_flow_instance.yaml` - new node type
- `crates/raisin-core/global_nodetypes/raisin_flow_step_execution.yaml` - new node type
- `crates/raisin-rocksdb/src/jobs/handlers/mod.rs` - register FlowExecution handler
- `crates/raisin-storage/src/jobs/types.rs` - add FlowExecution job type

**Key Features**:
- OCC for concurrency safety
- Parallel container (fork/join)
- Saga compensation stack for rollback
- Compensation execution on failure
- Metrics collection (duration, retries, etc.)

**Required Tests**:
- OCC prevents double-click problem
- Compensation executes in reverse order on failure
- Compensation failure handling (continue with others)
- Parallel step synchronization

**Observability (Day One)**:
- Flow execution logs in admin-console (extend existing function execution logs)
- Flow instance list: running, waiting, completed, failed
- Step-by-step execution history per instance
- Error display with stack trace
- Basic metrics stored on FlowInstance node

**Files to modify for observability**:
- `packages/admin-console/src/pages/` - add flow execution views
- Reuse existing execution log components from function logs

**Deliverable**: Can execute sequential AND parallel flows with saga-based rollback AND visual observability

### Phase 1.5: Error Handling & Retry (Critical for AI)
**Goal**: Robust error handling before AI integration

**Files to create/modify**:
- `crates/raisin-flow-runtime/src/runtime/retry.rs` - Exponential backoff logic
- `crates/raisin-flow-runtime/src/runtime/timeout.rs` - Timeout handling
- `crates/raisin-flow-runtime/src/handlers/error.rs` - Error classification and handling

**Implementation**:
- Step-level retry configuration (max_retries, backoff_strategy)
- Exponential backoff: 10s, 30s, 60s, 120s
- Timeout per step and per flow
- Error classification: retryable vs non-retryable
- Compensation stack tracking (prepare for saga rollback)

**Deliverable**: Flows can recover from transient failures (critical for LLM calls)

### Phase 2: AI Container
**Goal**: Replace ai-tools trigger-based approach with flow-based

**Files to create**:
- `crates/raisin-flow-runtime/src/handlers/ai_container.rs`
- `crates/raisin-flow-runtime/src/handlers/ai_container/tool_executor.rs`
- `crates/raisin-flow-runtime/src/handlers/ai_container/history_builder.rs`

**Files to modify**:
- `packages/raisin-flow-designer/src/types/flow.ts` - add AI container properties
- `packages/raisin-flow-designer/src/components/nodes/ContainerNode.tsx` - AI config panel
- `plugins/ai-tools/manifest.yaml` - provide template flow
- `plugins/ai-tools/content/flows/agent-conversation.yaml` - default flow

**Implementation**:
- AI container modes: auto, explicit, hybrid
- Tool call loop handling
- Conversation as child of FlowStepExecution
- Integration with raisin-ai crate

**Deliverable**: AI chat works via flow runtime, ai-tools triggers deprecated

### Phase 3: Human-in-the-Loop
**Goal**: Flows can pause for human input via inbox

**Files to create**:
- `crates/raisin-flow-runtime/src/handlers/human_task.rs`
- `crates/raisin-core/global_nodetypes/raisin_inbox_task.yaml`
- `crates/raisin-flow-runtime/src/runtime/cancellation.rs` - Propagate cancellation

**Files to modify**:
- `packages/admin-console/src/pages/` - inbox task UI
- `crates/raisin-rocksdb/src/jobs/handlers/` - trigger for task completion

**Implementation**:
- InboxTask node type
- Task creation in user inbox
- Response handling and flow resume
- Cancellation propagation to pending tasks
- Due date/timeout handling

**Deliverable**: Approval workflows work end-to-end with cancellation support

### Phase 4: Advanced Flow Designer Integration
**Goal**: Enhanced visual debugging and execution control

**Files to modify**:
- `packages/raisin-flow-designer/src/components/` - show instance state overlay on flow diagram
- `packages/admin-console/src/pages/flows/FlowDebugger.tsx` - step-through mode

**Implementation** (builds on Phase 1 observability):
- Visual indicator of current step ON the flow diagram
- Variable inspection at each step
- Step-through execution mode (pause/resume)
- Error highlighting with retry controls on diagram
- Live flow execution visualization

**Deliverable**: Full visual debugging integrated into flow designer

### Phase 5: Advanced Features (Future)
**Goal**: Production-ready features

- Sub-flows (execute nested flows)
- SLA monitoring and alerting
- Flow versioning UI (migrate running instances)
- Flow templates marketplace
- **Git-like branching (v2)**: Flow-level and per-step isolation
- Branch preview UI (inspect branch before merge)
- Merge conflict resolution UI
- Competing agents in parallel branches

---

## Development Notes

### Package Validation
When updating ai-tools plugin functions, validate with:
```bash
cd plugins
raisindb package create ai-tools --check
```
This validates all files in the package without creating the .rap file.

### Existing Infrastructure to Extend
- **Execution Logs**: `packages/admin-console/` already has execution logs for functions
  - Extend this for flow execution logs
  - Show flow instance → step → function execution hierarchy
  - Reuse existing log viewer components

---

## Example: Schedule Meeting Flow

User creates a "ScheduleMeetingRequest" node → triggers flow:

```yaml
# Flow definition
workflow_data:
  nodes:
    - id: start
      type: start
    - id: validate
      type: step
      function_ref: /lib/validate-meeting-request
    - id: check-calendar
      type: decision
      condition: "input.calendar_check_needed == true"
      yes: ai-schedule
      no: simple-schedule
    - id: ai-schedule
      type: ai_sequence
      properties:
        agent_ref: /agents/calendar-assistant
        tool_mode: auto
        max_iterations: 5
    - id: simple-schedule
      type: step
      function_ref: /lib/create-calendar-event
    - id: notify-participants
      type: step
      function_ref: /lib/send-notifications
    - id: end
      type: end

# Trigger
triggers:
  - trigger_type: node_event
    config:
      event_kinds: [Created]
    filters:
      node_types: [app:ScheduleMeetingRequest]
    function_flow_ref: /flows/templates/schedule-meeting
```

**Execution**:
1. User creates `app:ScheduleMeetingRequest` node
2. Trigger fires, creates `raisin:FlowInstance` in `/flows/instances/`
3. Runtime executes `validate` step (function job)
4. Runtime evaluates `check-calendar` decision
5. If yes → `ai-schedule` AI container starts conversation
   - Creates `raisin:AIConversation` as child of step execution
   - AI checks calendars, suggests times (tool calls)
   - Loop until AI is done
6. Runtime executes `notify-participants`
7. Flow completes

---

## Comparison: RaisinDB vs LangChain/LangFlow

| Feature | LangChain/LangFlow | RaisinDB Flow Runtime |
|---------|-------------------|----------------------|
| State Persistence | External (Redis, DB) | Native (nodes in database) |
| Versioning | Manual | Built-in (node versioning) |
| Rollback | Manual compensation | Saga compensation (structured) |
| Human-in-Loop | Custom code | Native inbox integration |
| Audit Trail | Logging | Full node history |
| Tool Execution | Inline (blocking) | Job queue with retry |
| Debugging | Print statements | Visual step-through |
| Parallel Execution | asyncio | Fork/join containers |
| Observability | External tools | Built-in metrics |
| Sub-flows | Manual | First-class support |
| Integration | API calls | Database triggers |

**Key differentiators**:
1. **Everything is a node** - Flow instances, steps, conversations, tool results are all queryable, versionable, and auditable
2. **Native human-in-loop** - Tasks in user inbox, flow pauses and resumes automatically
3. **Built-in retry & compensation** - Exponential backoff for LLM failures, saga rollback for side effects
4. **Observability by default** - Metrics, step history, visual debugging built-in
