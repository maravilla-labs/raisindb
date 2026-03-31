# @raisindb/flow-designer

Visual workflow designer component for RaisinDB.

## License

This project is licensed under the Business Source License 1.1 (BSL-1.1). See the LICENSE file in the repository root for details.

## Features

### Visual Flow Design

- **Drag & Drop** - Build workflows by dragging and dropping nodes
- **Node Types** - Start, End, Step, and Container nodes
- **Containers** - Sequence, Parallel, Conditional, Loop, Try/Catch, and AI containers
- **Connections** - Visual connectors between nodes with error paths

### Workflow Execution

- **Test Runs** - Execute workflows with mock data
- **Execution State** - Track node execution status in real-time
- **Error Handling** - Configure retry strategies and error behavior

### Developer Features

- **Command History** - Undo/redo support for all operations
- **Validation** - Real-time flow validation with issue reporting
- **Customizable** - Extensible node types and properties

## Installation

This is an internal package. Install from the monorepo:

```json
{
  "dependencies": {
    "@raisindb/flow-designer": "file:../raisin-flow-designer"
  }
}
```

## Usage

```tsx
import { FlowDesigner, type FlowDefinition } from '@raisindb/flow-designer';

function WorkflowEditor() {
  const [flow, setFlow] = useState<FlowDefinition>(createEmptyFlow());

  return (
    <FlowDesigner
      flow={flow}
      onChange={setFlow}
      onExecute={handleExecute}
    />
  );
}
```

## Exports

### Main Components

| Export | Description |
|--------|-------------|
| `FlowDesigner` | Main workflow designer component |
| `FlowCanvas` | Canvas for rendering flow nodes |
| `FlowToolbar` | Toolbar with flow actions |
| `NodePalette` | Palette for dragging new nodes |

### Node Components

| Export | Description |
|--------|-------------|
| `StartNode` | Flow start node |
| `EndNode` | Flow end node |
| `StepNode` | Action step node |
| `ContainerNode` | Container for grouping steps |
| `EmptyDropZone` | Drop target for empty containers |

### Hooks

| Export | Description |
|--------|-------------|
| `useDragAndDrop` | Drag and drop state management |
| `useCommandHistory` | Undo/redo command history |
| `useFlowState` | Flow state management |
| `useFlowExecution` | Flow execution control |
| `useFlowValidation` | Flow validation |
| `useSelection` | Node selection state |

### Commands

| Export | Description |
|--------|-------------|
| `AddStepCommand` | Add a new step to the flow |
| `DeleteStepCommand` | Remove a step from the flow |
| `MoveStepCommand` | Move a step to a new position |
| `UpdateStepCommand` | Update step properties |
| `UpdateRulesCommand` | Update container rules |
| `CommandHistory` | Command history manager |

### Types

```typescript
// Core flow types
FlowDefinition, FlowNode, FlowStep, FlowContainer

// Container types
ContainerType, ContainerRule, AiContainerConfig

// Execution types
ExecutionState, NodeExecutionStatus, FlowExecutionState

// Error handling
StepErrorBehavior, FlowErrorStrategy, RetryConfig
```

### Utilities

| Export | Description |
|--------|-------------|
| `findNodeById` | Find a node by ID |
| `cloneFlow` | Deep clone a flow definition |
| `createEmptyFlow` | Create a new empty flow |
| `calculateInsertPosition` | Calculate drop position |
| `generateNodeId` | Generate unique node IDs |

## Container Types

| Type | Description |
|------|-------------|
| `sequence` | Execute steps in order |
| `parallel` | Execute steps concurrently |
| `conditional` | Branch based on condition |
| `loop` | Repeat steps (for-each or while) |
| `try_catch` | Error handling with fallback |
| `ai` | AI-powered container with tool calling |

## Peer Dependencies

- `react` ^18.3.1
- `react-dom` ^18.3.1
