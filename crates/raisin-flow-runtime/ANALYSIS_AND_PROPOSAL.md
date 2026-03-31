# RaisinDB Flow Runtime: Production Readiness Analysis & Proposal

This document outlines the gaps identified in the current `raisin-flow-runtime` implementation and proposes architectural designs, algorithms, and missing components required to achieve a 100% production-ready state.

## 1. Executive Summary

The current runtime implements a solid foundation with:
- **Hybrid Batching & OCC**: Efficient execution with optimistic concurrency control.
- **Pluggable Architecture**: Handler-based design (`AiContainer`, `Decision`, etc.).
- **Integration Layer**: Event-driven triggers and job system integration.
- **Saga Pattern**: Basic compensation support.
- **Data Alignment**: The `FlowContext` structure correctly aligns with `raisin-functions` expectations via the `flow_input` field.

However, several critical flow patterns (`Loop`, `SubFlow`, `Join`) are currently unimplemented stubs. Additionally, advanced coordination patterns (async compensation, robust parallel joins) and data handling (formal mapping engine) need to be established to support complex enterprise workflows.

## 2. Critical Missing Implementations

The following step types are defined in `StepType` but unimplemented in `handlers/` and `integration/job_handler.rs`.

### 2.1. Loop / Iteration Handler (`StepType::Loop`)

**Gap:** No mechanism to iterate over collections or repeat steps based on conditions.

**Proposal:**
Implement a `LoopHandler` that manages iteration state within the `FlowInstance`.

*   **State Management:** Introduce a `LoopState` in `context_stack` (via `ContextFrame`) to track:
    *   `iterator_type`: `Collection` (foreach) or `Condition` (while/do-while).
    *   `collection_path`: Path to array in context (e.g., `step_outputs.search.results`).
    *   `current_index`: Current iteration index.
    *   `accumulator`: To store results from each iteration.
*   **Algorithm:**
    1.  **Init:** On first entry, resolve collection/condition using `DataMapper`. Push `LoopFrame` to stack.
    2.  **Execute:**
        *   If `index < length`:
            *   Update `context.loop_item` and `context.loop_index` variables.
            *   Return `StepResult::Continue` pointing to the *first child node* of the loop container.
        *   If done:
            *   Pop `LoopFrame`.
            *   Return `StepResult::Continue` pointing to `next_node` (exit loop).
    3.  **Child Completion:** When the last child of a loop iteration completes, the runtime execution loop must detect it is inside a `LoopFrame` and jump back to the **Loop Node** logic instead of the loop's `next_node`.

### 2.2. Sub-Flow Handler (`StepType::SubFlow`)

**Gap:** Ability to reuse workflows and isolate execution scopes.

**Proposal:**
Implement `SubFlowHandler` that spawns independent `FlowInstance`s linked to the parent.

*   **Lifecycle:**
    1.  **Spawn:** Parent creates Child Instance. Input is mapped from parent context using `DataMapper`.
    2.  **Link:** Child stores `parent_instance_ref`. Parent stores `child_instance_id` in `wait_info`.
    3.  **Wait:** Parent enters `FlowStatus::Waiting` (WaitType: `Event` / `SubFlowCompletion`).
    4.  **Resume:** When Child enters `Completed` state, the `FlowExecutionHandler` detects the `parent_instance_ref`. It queues a **Resume Job** for the parent, passing the child's `output` as the resume payload.
*   **Data Mapping:** Strict input/output mapping configuration to ensure encapsulation and prevent context pollution.

### 2.3. Join Gateway (`StepType::Join`)

**Gap:** `ParallelHandler` forks branches, but `Join` logic is currently a stub or relies on implicit completion.

**Proposal:**
Implement a "Barrier" pattern in the `state_manager` or `subscription` system.

*   **Mechanism:**
    *   Parent tracks `pending_branches` (List of Branch IDs) in its `ContextFrame` (Parallel type).
    *   When a parallel branch finishes (reaches a node pointing to the Join node), it doesn't just "stop". It updates the Parent's `pending_branches` list (atomic decrement/removal).
    *   **Wait Condition:** The `Join` step itself puts the flow in `Waiting`.
    *   **Trigger:** The completion of the *last* branch triggers the resume.

### 2.4. Wait / Delay (`StepType::Wait`)

**Gap:** Ability to pause execution for a specific duration or until a specific timestamp.

**Proposal:**
*   **Handler:** Calculates target `wake_up_time`. Returns `StepResult::Wait` with `WaitType::Scheduled`.
*   **Infrastructure:** Requires integration with the **Job System's Delayed Job** capabilities.
    *   The `FlowExecutionHandler` will queue a job with `execute_at = wake_up_time`.
*   **Resume:** When the job runs at the scheduled time, it triggers a `TriggerEventType::Scheduled` event which resumes the flow.

## 3. Architectural Enhancements

### 3.1. Robust Data Mapping Engine (`DataMapper`)

**Problem:** Current property access is ad-hoc (`get_string_property`, etc.).
**Solution:** Introduce a standard `DataMapper` struct.

*   **Syntax:**
    *   Literal values: `123`, `"string"`
    *   Context References: `${input.user.name}`, `${steps.step_1.result}`
    *   Expressions: `${input.price * 1.2}` (using `raisin-rel`)
*   **Implementation:**
    ```rust
    pub struct DataMapper;
    impl DataMapper {
        pub fn map(value: &Value, context: &FlowContext) -> FlowResult<Value> { ... }
    }
    ```
*   **Integration:** All handlers (Function, AI, SubFlow) must use `DataMapper` to resolve their configuration properties (arguments, inputs) before execution.

### 3.2. Asynchronous Compensation

**Problem:** `rollback_flow` executes compensations synchronously (`execute_function`).
**Gap:** Real-world compensations often need to wait (e.g., "Request Refund" -> Wait for Payment Provider).
**Proposal:**
*   **State:** Refactor `CompensationEntry` to support `StepType::AIContainer` or `StepType::HumanTask` as compensations.
*   **Execution:** If a compensation returns `StepResult::Wait`, the Rollback process itself must suspend and enter a `RollingBack` state (distinct from `Running` or `Failed`), waiting for the compensation to finish before continuing the LIFO stack unwinding.

### 3.3. Flow Control & Quotas

**Problem:** Infinite loops or massive parallel fan-outs can crash the system.
**Proposal:**
*   **Max Execution Depth:** Limit `context_stack` depth (e.g., 50).
*   **Max Steps per Instance:** Hard limit on `metrics.step_count` (e.g., 10,000) to prevent runaway loops.
*   **Rate Limiting:** `FlowExecutionHandler` should check tenant quotas before loading/executing steps.

## 4. Implementation Roadmap

1.  **Phase 1: Structure & Data**
    *   Implement `DataMapper` utility.
    *   Refactor `FunctionStepHandler` and `AiContainerHandler` to use `DataMapper`.

2.  **Phase 2: Control Flow**
    *   Implement `LoopHandler` and `StepType::Loop`.
    *   Implement `SubFlowHandler` and parent/child linkage logic.

3.  **Phase 3: Parallel & Sync**
    *   Refactor `ParallelHandler` to use robust Branch State tracking.
    *   Implement `JoinHandler`.

4.  **Phase 4: Robustness**
    *   Implement Async Compensation support.
    *   Add execution limits (depth, step count).