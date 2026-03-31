
# RaisinDB AI/Agentic Workflow Enhancement Roadmap

*Generated from brainstorming session comparing RaisinDB with pydantic-ai*

---

## Current State Comparison

### What RaisinDB Has Well
- **Node-based conversation persistence** - Full audit trail as a tree structure
- **Parallel tool execution** with aggregation (AIToolResultAggregator)
- **Flow runtime** - Full saga pattern with compensation/rollback
- **Human-in-the-loop** - HumanTask steps, AITask/AIPlan nodes
- **Trigger-based execution** - Event-driven agent loop
- **Job queue integration** - Async function execution
- **Token tracking** - Per-message and conversation-level

### Potential Blind Spots / Enhancement Areas

| Area | Pydantic-AI | RaisinDB Current | Gap |
|------|-------------|------------------|-----|
| **Structured Output Validation** | 3 modes: ToolOutput, NativeOutput, PromptedOutput + validators | Basic JSON schema in output_schema | No runtime validation, no multiple strategies |
| **Streaming** | Rich streaming (text, response, output with validation) | None apparent | Major gap for UX |
| **Model Retry Pattern** | `ModelRetry` exception for self-correction | Tool retry via job queue | No "ask model to fix itself" pattern |
| **Output Validators** | `@agent.output_validator` decorators | JSON schema only | No custom validation logic |
| **Dynamic Tools** | Prepare functions, dynamic toolsets | Static tool references | Tools can't be contextual |
| **History Processors** | Filter/transform history before model call | None | Can't summarize/trim context |
| **End Strategies** | 'early' vs 'exhaustive' tool execution | Continue until no tool calls | No explicit strategy control |
| **Evaluation Framework** | pydantic_evals with datasets, evaluators | None | No systematic testing |
| **Observability** | OpenTelemetry + Logfire | Basic metrics | No tracing integration |
| **Multi-modal** | ImageUrl, AudioUrl, VideoUrl, DocumentUrl | Attachments array | Less structured |
| **Deferred Approval** | ApprovalRequired in tool flow | Separate HumanTask steps | Less integrated |
| **Dependency Injection** | Type-safe RunContext[AgentDepsT] | Workspace context | Less type-safe |
| **Timeout Granularity** | Agent-level + per-tool overrides | resource_limits on function | Less flexible |

## Brainstorming Topics

### 1. Streaming Support
- Real-time token streaming for UX
- Partial structured output updates
- Progress events during tool execution

### 2. Output Validation Strategies
- Tool-based output (current implicit)
- Native structured output (Claude, GPT-4)
- Prompted output extraction
- Custom validator functions

### 3. Self-Correction Patterns
- ModelRetry mechanism for validation failures
- Asking model to fix its own output
- Configurable retry prompts

### 4. Dynamic Tool Selection
- Tools that adapt based on conversation context
- Per-step tool availability
- Tool "prepare" phase

### 5. History Management
- Context window management
- History summarization
- Selective message inclusion

### 6. Evaluation & Testing
- Dataset-driven agent testing
- Custom evaluators
- Regression testing

### 7. Observability
- OpenTelemetry spans
- Distributed tracing
- Cost tracking dashboards

### 8. Approval Workflows
- In-tool approval (vs separate HumanTask)
- Approval metadata/context
- Batch approvals

## User Context
- **Use cases**: Multi-step workflows, Internal automation, Customer-facing chatbots
- **All gaps important**: Self-correction, Evaluation, Output validation, Streaming
- **Existing streaming**: WebSocket events for node changes, SSE for admin-console (not in raisin-client-js)

---

## Deep Dive: Enhancement Ideas

### 1. STREAMING SUPPORT

**Current State:**
- `CompletionRequest.stream: bool` exists
- Provider trait has `supports_streaming()`
- WebSocket events stream node changes to clients
- Gap: Streaming not exposed through JS function API

**Enhancement Options:**

**Option A: Stream-to-Events (Recommended)**
```
AI Provider → Rust runtime → AIMessage node (with partial content)
                          → WebSocket event (content_delta)
                          → Client receives chunks
```
- Create AIMessage immediately with `status: streaming`
- Emit `content_delta` events as tokens arrive
- Final update sets `status: complete`
- Works with existing WebSocket infrastructure

**Option B: Dedicated Stream API**
- New `raisin.ai.completionStream()` returning async iterator
- Requires QuickJS async iterator support
- More complex but more flexible

**Option C: SSE passthrough**
- Expose SSE endpoint that proxies provider stream
- Client connects directly for streaming
- Bypasses node system

### 2. OUTPUT VALIDATION STRATEGIES

**Current State:**
- `output_schema` in raisin:Function (JSON schema)
- No runtime validation library
- Validation delegated to AI providers

**Enhancement Options:**

**A. Add jsonschema-rs crate** (Recommended)
```rust
// In function execution
let output = execute_function(...);
if let Some(schema) = function.output_schema {
    jsonschema::validate(&schema, &output)?;
}
```

**B. Multiple Output Modes** (like pydantic-ai)
```yaml
# In raisin:AIAgent
output_strategy: tool | native | prompted
output_schema: { ... }
output_validators: ["/functions/validators/check-output"]
```

- **Tool mode**: Current approach - AI calls a "respond" tool
- **Native mode**: Use provider's structured output (Claude, GPT-4)
- **Prompted mode**: Add schema to system prompt, parse response

**C. Custom Validators**
```yaml
# In raisin:Function
output_validators:
  - ref: workspace:/functions/validators/validate-order
  - ref: workspace:/functions/validators/check-business-rules
```
- Chain of validator functions
- Each can modify or reject output

### 3. SELF-CORRECTION (ModelRetry Pattern)

**Current State:**
- Tool failures retry via job queue
- No "ask model to fix itself" pattern

**Enhancement Ideas:**

**A. ValidationRetry Exception**
```javascript
// In tool or validator
if (!isValid(output)) {
    throw new raisin.ValidationRetry(
        "Output missing required field 'customer_id'",
        { hint: "Include the customer ID from the input" }
    );
}
```
- Agent handler catches ValidationRetry
- Appends retry message to conversation
- Re-calls AI with error context
- Configurable max_retries per agent

**B. Output Validator with Retry**
```yaml
# In raisin:AIAgent
output_validation:
  schema: { ... }
  on_failure: retry | fail | warn
  retry_prompt: "Your output didn't match the schema: {error}"
  max_retries: 3
```

**C. Tool-Level Self-Correction**
```yaml
# In raisin:Function
on_error:
  strategy: retry_with_feedback | fail | fallback
  retry_prompt: "The tool failed: {error}. Please try a different approach."
  max_retries: 2
```

### 4. EVALUATION FRAMEWORK

**Current State:**
- No systematic testing infrastructure
- Can manually test via conversations

**Enhancement Ideas:**

**A. EvalDataset Node Type**
```yaml
name: raisin:EvalDataset
properties:
  - name: name
  - name: description
  - name: agent_ref  # Agent to test
allowed_children:
  - raisin:EvalCase
```

**B. EvalCase Node Type**
```yaml
name: raisin:EvalCase
properties:
  - name: input
    type: String  # User message
  - name: expected_output
    type: Object  # Expected structure/content
  - name: evaluators
    type: Array  # List of evaluator refs
  - name: tags
    type: Array  # For filtering
```

**C. Built-in Evaluators**
- `contains_text` - Output contains expected text
- `matches_schema` - Output matches JSON schema
- `semantic_similarity` - Embedding similarity score
- `llm_judge` - Use another LLM to evaluate
- `custom_function` - Reference a validator function

**D. Eval Run & Reporting**
```yaml
name: raisin:EvalRun
properties:
  - name: dataset_ref
  - name: status  # pending, running, completed
  - name: results  # Array of case results
  - name: summary  # Aggregated metrics
  - name: started_at
  - name: completed_at
```

### 5. DYNAMIC TOOL SELECTION

**Current State:**
- Static `tools` array on AIAgent
- All tools always available

**Enhancement Ideas:**

**A. Tool Conditions**
```yaml
# In raisin:AIAgent
tools:
  - ref: workspace:/functions/tools/weather
    condition: "input.location != null"
  - ref: workspace:/functions/tools/search
    condition: "variables.search_enabled"
```

**B. Tool Prepare Phase**
```yaml
# In raisin:AIAgent
tool_preparer: workspace:/functions/prepare-tools
# Function receives context, returns filtered tool list
```

**C. Contextual Toolsets**
```yaml
# In raisin:AIAgent
toolsets:
  - name: basic
    tools: [...]
    default: true
  - name: admin
    tools: [...]
    condition: "user.role == 'admin'"
```

### 6. HISTORY MANAGEMENT

**Current State:**
- Full history rebuilt from conversation tree
- No trimming or summarization

**Enhancement Ideas:**

**A. History Processor Functions**
```yaml
# In raisin:AIAgent
history_processors:
  - ref: workspace:/functions/processors/trim-old-messages
  - ref: workspace:/functions/processors/summarize-long-conversations
```

**B. Built-in Processors**
- `max_messages(n)` - Keep last N messages
- `max_tokens(n)` - Trim to fit context window
- `summarize_after(n)` - Summarize messages older than N
- `keep_system_and_last(n)` - System prompt + last N

**C. Smart Context Window**
```yaml
# In raisin:AIAgent
context_management:
  strategy: sliding_window | summarize | truncate
  max_tokens: 8000
  summarize_threshold: 6000
  keep_recent: 5
```

### 7. MULTI-MODEL FALLBACKS

**Architectural Fit**: Functions + Flows as integration points

**Option A: Flow-Level Fallback (Alternative Steps)**
```yaml
# In flow definition
nodes:
  - id: primary_ai
    step_type: AIContainer
    properties:
      agent_ref: /agents/main-agent
      model: gpt-4o
    on_error:
      goto: fallback_ai

  - id: fallback_ai
    step_type: AIContainer
    properties:
      agent_ref: /agents/main-agent
      model: claude-3-5-sonnet
```
- Explicit in flow definition
- Full control over fallback logic
- Can have multiple fallback levels

**Option B: Step-Level Config (In Properties)**
```yaml
# In AIContainer step
properties:
  agent_ref: /agents/main-agent
  model_fallbacks:
    - model: gpt-4o
      priority: 1
    - model: claude-3-5-sonnet
      priority: 2
      on_errors: [rate_limit, timeout, unavailable]
    - model: gpt-4o-mini
      priority: 3
      on_errors: [*]  # catch-all
```
- Simpler for common patterns
- Automatic retry with different models
- Configurable error triggers

**Option C: Agent-Level Default Fallbacks**
```yaml
# In raisin:AIAgent
properties:
  model: gpt-4o
  fallback_models:
    - claude-3-5-sonnet
    - gpt-4o-mini
  fallback_strategy: sequential | fastest | cheapest
```
- Applied across all uses of agent
- Tenant-level default fallback chain

### 8. GUARDRAILS & SAFETY

**Input Guardrails (Before AI Call)**
```yaml
# In raisin:AIAgent
input_guardrails:
  - type: content_filter
    config:
      block_categories: [hate, violence, self_harm]
  - type: pii_detector
    config:
      action: redact | warn | block
      types: [email, phone, ssn, credit_card]
  - type: custom_function
    ref: workspace:/functions/guardrails/check-input
```

**Output Guardrails (After AI Response)**
```yaml
output_guardrails:
  - type: toxicity_check
    config:
      threshold: 0.7
      on_fail: retry | redact | block
  - type: pii_detector
    config:
      action: redact
  - type: schema_compliance
    schema: { ... }
  - type: custom_function
    ref: workspace:/functions/guardrails/check-output
```

**Tool Guardrails (Before Tool Execution)**
```yaml
# In raisin:Function
guardrails:
  - type: rate_limit
    config:
      max_calls_per_minute: 10
  - type: parameter_validation
    # Uses input_schema
  - type: approval_required
    config:
      condition: "args.action == 'delete'"
```

**Integration with Flows**
- Guardrail failures can trigger flow error handling
- Compensation for blocked operations
- Audit trail via AIMessage/AIToolCall nodes

### 9. MULTI-AGENT HANDOFFS

**Option A: Agent-as-Tool Pattern**
```yaml
# In raisin:AIAgent (orchestrator)
tools:
  - ref: workspace:/functions/agents/specialist-agent
    description: "Delegate to specialist for complex calculations"
```

```javascript
// specialist-agent function
export async function handler(ctx) {
  const { query } = ctx.input;
  const conversation = await raisin.nodes.create(ctx.workspace, {
    node_type: 'raisin:AIConversation',
    properties: {
      agent_ref: 'workspace:/agents/specialist',
      // ...
    }
  });
  // Create user message, wait for response
  // Return result to orchestrator
}
```

**Option B: Flow-Based Orchestration**
```yaml
# Multi-agent flow
nodes:
  - id: triage
    step_type: AIContainer
    properties:
      agent_ref: /agents/triage-agent
      # Returns: { route: "billing" | "technical" | "sales" }

  - id: route_decision
    step_type: Decision
    properties:
      condition: "output.route"
      branches:
        billing: billing_agent
        technical: tech_agent
        sales: sales_agent

  - id: billing_agent
    step_type: AIContainer
    properties:
      agent_ref: /agents/billing-specialist
```

**Option C: Nested Conversations**
```yaml
# In raisin:AIAgent
properties:
  delegate_agents:
    - name: code_review
      agent_ref: workspace:/agents/code-reviewer
      trigger: "needs code review"
    - name: research
      agent_ref: workspace:/agents/researcher
      trigger: "needs research"
```
- AI decides when to delegate
- Child conversation created automatically
- Result injected back into parent

**Context Sharing**
```yaml
# Shared context across agents
handoff_context:
  include: [user_profile, conversation_summary, current_task]
  exclude: [internal_reasoning, tool_calls]
  max_tokens: 2000
```

### 10. MCP (Model Context Protocol) INTEGRATION

**Architectural Fit**: Treat MCP servers like external function providers

**Option A: MCP as Tool Source**
```yaml
# New node type: raisin:MCPConnection
name: raisin:MCPConnection
properties:
  - name: name
    type: String
  - name: transport
    type: String  # stdio | http | sse
  - name: command
    type: String  # For stdio: "npx -y @anthropic/mcp-server-github"
  - name: url
    type: String  # For http/sse
  - name: env
    type: Object  # Environment variables
  - name: enabled
    type: Boolean
```

**Agent Integration**
```yaml
# In raisin:AIAgent
properties:
  tools:
    - ref: workspace:/functions/local-tool
    - ref: workspace:/mcp/github  # MCP connection reference
  mcp_tool_filter:  # Optional: limit which MCP tools are exposed
    include: [search_repositories, create_issue]
    exclude: [delete_*]
```

**Runtime Flow**
1. Agent handler loads MCP connections
2. Connects to MCP server (lazy, cached)
3. Fetches available tools via MCP protocol
4. Merges with local function tools
5. On tool call: Routes to MCP server or local executor
6. MCP result converted to AIToolResult

**MCP in Flows**
```yaml
# Flow step that uses MCP
- id: github_search
  step_type: FunctionStep
  properties:
    function_ref: mcp://github/search_repositories
    # OR
    mcp_connection: workspace:/mcp/github
    mcp_tool: search_repositories
```

**MCP Server Lifecycle**
- Start on first use (lazy)
- Keep alive with heartbeat
- Reconnect on failure
- Shutdown on tenant deactivation
- Pool management for high-traffic

### 11. MEMORY & RAG INTEGRATION

**Current State (from exploration):**
- RaisinDB has vector/embedding support in raisin-ai
- Node tree can serve as conversation memory

**Long-Term Memory Architecture:**

```yaml
# New node type: raisin:AIMemory
name: raisin:AIMemory
properties:
  - name: type
    type: String  # episodic | semantic | procedural
  - name: content
    type: String
  - name: embedding
    type: Array  # Vector embedding
  - name: importance
    type: Number  # 0-1 for relevance scoring
  - name: last_accessed
    type: Date
  - name: access_count
    type: Number
  - name: metadata
    type: Object  # source conversation, context
```

**Agent Memory Integration:**
```yaml
# In raisin:AIAgent
memory:
  enabled: true
  store: workspace:/memory  # Where memories are stored
  retrieval:
    strategy: semantic | recency | hybrid
    max_memories: 10
    min_similarity: 0.7
  persistence:
    auto_save: true  # Save memories from conversations
    importance_threshold: 0.5
```

**Memory in History Building:**
```javascript
// In agent-handler
async function buildHistory(conversation, agent) {
  const history = [];

  // 1. System prompt
  history.push({ role: 'system', content: agent.system_prompt });

  // 2. Relevant memories (RAG)
  if (agent.memory?.enabled) {
    const memories = await retrieveMemories(
      conversation.latest_message,
      agent.memory
    );
    if (memories.length > 0) {
      history.push({
        role: 'system',
        content: formatMemories(memories)
      });
    }
  }

  // 3. Conversation messages
  // ... existing history building
}
```

**Memory Types:**
- **Episodic**: Past conversation snippets, user preferences
- **Semantic**: Learned facts, domain knowledge
- **Procedural**: Successful tool patterns, workflows

**RAG Query Flow:**
```
User Message
    → Embed query
    → Search AIMemory nodes (vector similarity)
    → Rank by (similarity * importance * recency_decay)
    → Inject top-k into context
    → AI generates response
    → Extract & save new memories (optional)
```

### 12. COST OPTIMIZATION

**Token Budget Management:**
```yaml
# In raisin:AIAgent or AIConversation
cost_limits:
  max_tokens_per_message: 4000
  max_tokens_per_conversation: 50000
  max_cost_per_conversation: 1.00  # USD
  on_limit_exceeded: warn | truncate | fail
```

**Caching Strategies:**

**A. Response Caching (Deterministic Queries)**
```yaml
# In raisin:AIAgent
caching:
  enabled: true
  cache_key: hash(system_prompt + messages[-1])
  ttl: 3600  # seconds
  temperature_threshold: 0  # Only cache when temp=0
```

**B. Prompt Caching (Claude's Feature)**
```yaml
# Automatically use Claude's prompt caching for long system prompts
provider_features:
  anthropic:
    prompt_caching: true  # Cache system prompt tokens
```

**C. Embedding Cache**
```yaml
embedding_cache:
  enabled: true
  provider: redis | memory | rocksdb
  ttl: 86400
```

**Batching:**
```yaml
# For high-volume scenarios
batching:
  enabled: true
  max_batch_size: 10
  max_wait_ms: 100
  # Group similar requests to single API call
```

**Cost Tracking:**
```yaml
# New node type: raisin:AICostRecord
properties:
  - name: conversation_ref
  - name: agent_ref
  - name: model
  - name: input_tokens
  - name: output_tokens
  - name: cost_usd
  - name: timestamp
```

**Budget Alerts:**
```yaml
# In tenant config
cost_alerts:
  - threshold: 100  # USD
    action: email
  - threshold: 500
    action: pause_new_conversations
```

### 13. A/B TESTING AGENTS

**Agent Variants:**
```yaml
# New node type: raisin:AIAgentExperiment
properties:
  - name: name
  - name: status  # draft | running | completed | paused
  - name: variants
    type: Array
    # Each variant references an AIAgent
  - name: traffic_split
    type: Object  # { "control": 50, "variant_a": 50 }
  - name: metrics
    type: Array  # What to measure
  - name: start_date
  - name: end_date
  - name: min_conversations  # Statistical significance
```

**Experiment Assignment:**
```javascript
// In agent selection
async function selectAgent(experimentRef, userId) {
  const experiment = await raisin.nodes.get(experimentRef);

  if (experiment.status !== 'running') {
    return experiment.control_agent;
  }

  // Deterministic assignment (user always gets same variant)
  const bucket = hash(userId + experiment.id) % 100;
  let cumulative = 0;

  for (const [variant, percentage] of Object.entries(experiment.traffic_split)) {
    cumulative += percentage;
    if (bucket < cumulative) {
      return experiment.variants[variant];
    }
  }
}
```

**Metrics Collection:**
```yaml
# Tracked automatically per conversation
metrics:
  - name: completion_rate
    description: User completed their goal
  - name: turns_to_resolution
    description: Number of back-and-forth messages
  - name: tool_call_success_rate
  - name: user_rating
    description: Explicit thumbs up/down
  - name: latency_p50
  - name: cost_per_conversation
```

**Results Analysis:**
```yaml
# In raisin:AIAgentExperiment.results
results:
  control:
    conversations: 500
    metrics:
      completion_rate: 0.72
      turns_to_resolution: 4.2
  variant_a:
    conversations: 500
    metrics:
      completion_rate: 0.81
      turns_to_resolution: 3.5
  statistical_significance: 0.95
  recommendation: "variant_a outperforms control"
```

---

## DEEP DIVE: STREAMING IMPLEMENTATION

### Architecture Decision: Stream-to-Events

```
┌─────────────────────────────────────────────────────────────┐
│                      Client (Browser)                        │
│   WebSocket ← events (content_delta, message_complete)       │
└─────────────────────────────────────────────────────────────┘
                              ↑
┌─────────────────────────────────────────────────────────────┐
│                      Event Bus                               │
│   emit("ai.content_delta", { message_id, delta, ... })       │
└─────────────────────────────────────────────────────────────┘
                              ↑
┌─────────────────────────────────────────────────────────────┐
│                   Agent Handler (Rust)                       │
│   1. Create AIMessage (status: streaming)                    │
│   2. Call provider with stream=true                          │
│   3. For each chunk:                                         │
│      - Emit content_delta event                              │
│      - Accumulate content                                    │
│   4. Update AIMessage (status: complete, full content)       │
│   5. Emit message_complete event                             │
└─────────────────────────────────────────────────────────────┘
                              ↑
┌─────────────────────────────────────────────────────────────┐
│                   AI Provider (tokio stream)                 │
│   OpenAI: SSE stream → delta chunks                          │
│   Anthropic: SSE stream → delta chunks                       │
│   Gemini: SSE stream → delta chunks                          │
└─────────────────────────────────────────────────────────────┘
```

### Key Implementation Points

**1. AIMessage Status Extension:**
```yaml
# In ai_message.yaml
properties:
  - name: status
    type: String
    enum: [pending, streaming, complete, failed]
    default: pending
```

**2. New Event Types:**
```rust
// In event system
pub enum AIEvent {
    ContentDelta {
        message_id: String,
        delta: String,           // Incremental text
        accumulated: String,     // Full text so far
        index: u32,              // Chunk index
    },
    MessageComplete {
        message_id: String,
        finish_reason: String,
        usage: TokenUsage,
    },
    ToolCallStart {
        message_id: String,
        tool_call_id: String,
        function_name: String,
    },
    ToolCallArgumentsDelta {
        tool_call_id: String,
        delta: String,
    },
}
```

**3. Provider Streaming Trait:**
```rust
// In raisin-ai
pub trait StreamingProvider: AIProviderTrait {
    async fn complete_streaming(
        &self,
        request: CompletionRequest,
    ) -> Result<impl Stream<Item = Result<StreamChunk, Error>>>;
}

pub enum StreamChunk {
    ContentDelta(String),
    ToolCallStart { id: String, name: String },
    ToolCallArguments { id: String, delta: String },
    Done { finish_reason: String, usage: TokenUsage },
}
```

**4. Agent Handler Changes (Rust side):**
```rust
async fn handle_user_message_streaming(
    ctx: &ExecutionContext,
    message: &AIMessage,
) -> Result<()> {
    // 1. Create response message with streaming status
    let response_id = create_ai_message(ctx, AIMessageStatus::Streaming)?;

    // 2. Build request with streaming enabled
    let request = build_completion_request(ctx, message)
        .with_streaming(true);

    // 3. Get streaming response
    let stream = provider.complete_streaming(request).await?;

    // 4. Process stream
    let mut accumulated = String::new();
    let mut index = 0;

    while let Some(chunk) = stream.next().await {
        match chunk? {
            StreamChunk::ContentDelta(delta) => {
                accumulated.push_str(&delta);
                ctx.emit_event(AIEvent::ContentDelta {
                    message_id: response_id.clone(),
                    delta,
                    accumulated: accumulated.clone(),
                    index,
                })?;
                index += 1;
            }
            StreamChunk::Done { finish_reason, usage } => {
                update_ai_message(ctx, &response_id, AIMessageStatus::Complete, &accumulated)?;
                ctx.emit_event(AIEvent::MessageComplete {
                    message_id: response_id,
                    finish_reason,
                    usage,
                })?;
                break;
            }
            // Handle tool calls...
        }
    }

    Ok(())
}
```

**5. Client Integration (raisin-client-js):**
```typescript
// New event subscriptions
client.events.on('ai.content_delta', (event) => {
  const { message_id, delta, accumulated } = event.data;
  // Update UI with streaming content
  updateMessageContent(message_id, accumulated);
});

client.events.on('ai.message_complete', (event) => {
  const { message_id, finish_reason, usage } = event.data;
  // Mark message as complete
  finalizeMessage(message_id);
});
```

### Streaming with Tool Calls

When AI responds with tool calls during streaming:
1. Stream text content as deltas
2. Emit `ToolCallStart` when tool call begins
3. Stream tool call arguments as deltas
4. On stream end: create AIToolCall nodes, trigger execution
5. Tool results trigger agent-continue-handler (existing flow)

---

## DEEP DIVE: MCP IMPLEMENTATION

### Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    RaisinDB Core                             │
└─────────────────────────────────────────────────────────────┘
           ↓                              ↓
┌─────────────────────┐      ┌─────────────────────────────────┐
│  raisin:Function    │      │  raisin:MCPConnection           │
│  (Native tools)     │      │  (External MCP servers)         │
└─────────────────────┘      └─────────────────────────────────┘
           ↓                              ↓
┌─────────────────────────────────────────────────────────────┐
│                   Tool Registry                              │
│   - Local functions                                          │
│   - MCP server tools (discovered dynamically)                │
└─────────────────────────────────────────────────────────────┘
           ↓
┌─────────────────────────────────────────────────────────────┐
│                   Agent Handler                              │
│   Merges all tool sources → AI completion                    │
└─────────────────────────────────────────────────────────────┘
```

### Node Type Definition

```yaml
name: raisin:MCPConnection
description: Connection to an MCP (Model Context Protocol) server
icon: plug
version: 1
strict: true
properties:
  - name: name
    type: String
    required: true
    title: Connection Name
    description: Unique identifier for this MCP connection
    index: [Property]

  - name: transport
    type: String
    required: true
    title: Transport Type
    enum: [stdio, sse, websocket]
    default: stdio

  - name: command
    type: String
    required: false
    title: Command (stdio)
    description: "Shell command to start server (e.g., 'npx -y @modelcontextprotocol/server-github')"

  - name: url
    type: String
    required: false
    title: URL (sse/websocket)
    description: Server URL for HTTP-based transports

  - name: env
    type: Object
    required: false
    title: Environment Variables
    description: "Environment variables for the server process"

  - name: enabled
    type: Boolean
    required: false
    default: true

  - name: health_check_interval
    type: Number
    required: false
    default: 30
    description: Seconds between health checks

  - name: timeout_ms
    type: Number
    required: false
    default: 30000
    description: Request timeout

  - name: tool_filter
    type: Object
    required: false
    description: "{ include: ['tool1', 'tool2'], exclude: ['tool3'] }"

  - name: cached_tools
    type: Array
    required: false
    title: Cached Tool Definitions
    description: Tool definitions from last discovery (auto-populated)

  - name: last_connected
    type: Date
    required: false

  - name: status
    type: String
    enum: [disconnected, connecting, connected, error]
    default: disconnected
```

### MCP Client Implementation (Rust)

```rust
// New crate: raisin-mcp

pub struct MCPClient {
    transport: Box<dyn MCPTransport>,
    capabilities: ServerCapabilities,
    tools: Vec<MCPToolDefinition>,
}

#[async_trait]
pub trait MCPTransport: Send + Sync {
    async fn send(&self, request: JSONRPCRequest) -> Result<JSONRPCResponse>;
    async fn close(&self) -> Result<()>;
}

pub struct StdioTransport {
    child: tokio::process::Child,
    stdin: tokio::io::BufWriter<ChildStdin>,
    stdout: tokio::io::BufReader<ChildStdout>,
}

pub struct SSETransport {
    client: reqwest::Client,
    endpoint: String,
}

impl MCPClient {
    pub async fn connect(config: &MCPConnectionConfig) -> Result<Self> {
        let transport = match config.transport {
            TransportType::Stdio => {
                StdioTransport::spawn(&config.command, &config.env).await?
            }
            TransportType::SSE => {
                SSETransport::connect(&config.url).await?
            }
            TransportType::WebSocket => {
                WebSocketTransport::connect(&config.url).await?
            }
        };

        // Initialize connection
        let init_response = transport.send(JSONRPCRequest::initialize()).await?;
        let capabilities = init_response.capabilities;

        // Discover tools
        let tools_response = transport.send(JSONRPCRequest::list_tools()).await?;
        let tools = tools_response.tools;

        Ok(Self { transport, capabilities, tools })
    }

    pub async fn call_tool(&self, name: &str, args: Value) -> Result<Value> {
        let response = self.transport.send(JSONRPCRequest::call_tool(name, args)).await?;
        Ok(response.result)
    }

    pub fn get_tool_definitions(&self) -> &[MCPToolDefinition] {
        &self.tools
    }
}
```

### Integration with Agent Handler

```rust
// In agent handler
async fn resolve_tools(ctx: &Context, agent: &AIAgent) -> Result<Vec<ToolDefinition>> {
    let mut tools = Vec::new();

    // 1. Load native raisin:Function tools
    for tool_ref in &agent.tools {
        if tool_ref.starts_with("mcp://") {
            continue; // Handle MCP separately
        }
        let func = load_function(ctx, tool_ref).await?;
        tools.push(func.to_tool_definition());
    }

    // 2. Load MCP tools
    for tool_ref in &agent.tools {
        if let Some(mcp_ref) = tool_ref.strip_prefix("mcp://") {
            let (connection_name, tool_name) = parse_mcp_ref(mcp_ref)?;
            let client = get_or_connect_mcp(ctx, connection_name).await?;

            if let Some(tool_name) = tool_name {
                // Specific tool
                if let Some(tool) = client.get_tool(tool_name) {
                    tools.push(tool.to_ai_tool_definition());
                }
            } else {
                // All tools from this connection
                for tool in client.get_tool_definitions() {
                    if should_include_tool(tool, &agent.mcp_tool_filter) {
                        tools.push(tool.to_ai_tool_definition());
                    }
                }
            }
        }
    }

    Ok(tools)
}

// Tool execution routing
async fn execute_tool_call(
    ctx: &Context,
    tool_call: &AIToolCall,
) -> Result<Value> {
    let tool_name = &tool_call.function_name;

    // Check if this is an MCP tool
    if let Some(mcp_route) = ctx.mcp_tool_routes.get(tool_name) {
        let client = get_mcp_client(ctx, &mcp_route.connection).await?;
        return client.call_tool(tool_name, tool_call.arguments.clone()).await;
    }

    // Otherwise, execute as native function
    execute_function(ctx, tool_call).await
}
```

### Connection Pool Management

```rust
pub struct MCPConnectionPool {
    connections: DashMap<String, Arc<MCPClient>>,
    config: PoolConfig,
}

impl MCPConnectionPool {
    pub async fn get_or_connect(&self, name: &str, config: &MCPConnectionConfig) -> Result<Arc<MCPClient>> {
        // Check existing connection
        if let Some(client) = self.connections.get(name) {
            if client.is_healthy().await {
                return Ok(client.clone());
            }
            // Connection unhealthy, remove and reconnect
            self.connections.remove(name);
        }

        // Create new connection
        let client = Arc::new(MCPClient::connect(config).await?);
        self.connections.insert(name.to_string(), client.clone());

        // Start health check task
        self.spawn_health_checker(name.to_string(), client.clone());

        Ok(client)
    }

    fn spawn_health_checker(&self, name: String, client: Arc<MCPClient>) {
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                if !client.ping().await.is_ok() {
                    // Mark as unhealthy, will reconnect on next use
                    client.mark_unhealthy();
                    break;
                }
            }
        });
    }
}
```

---

## DEEP DIVE: EVALUATION FRAMEWORK

### Node Type Schemas

**EvalDataset:**
```yaml
name: raisin:EvalDataset
description: Collection of test cases for evaluating an AI agent
icon: clipboard-check
version: 1
properties:
  - name: name
    type: String
    required: true
    index: [Fulltext]

  - name: description
    type: String
    required: false

  - name: agent_ref
    type: Reference
    required: true
    allowed_types: [raisin:AIAgent]
    description: Agent to evaluate

  - name: status
    type: String
    enum: [draft, active, archived]
    default: draft

  - name: default_evaluators
    type: Array
    required: false
    description: "Evaluators applied to all cases unless overridden"

  - name: tags
    type: Array
    required: false
    description: "Tags for filtering datasets"

  - name: created_by
    type: String
    required: false

  - name: metadata
    type: Object
    required: false

allowed_children:
  - raisin:EvalCase
```

**EvalCase:**
```yaml
name: raisin:EvalCase
description: Single test case for agent evaluation
icon: test-tube
version: 1
properties:
  - name: name
    type: String
    required: true

  - name: description
    type: String
    required: false

  - name: input
    type: String
    required: true
    title: User Input
    description: Message to send to agent

  - name: input_context
    type: Object
    required: false
    title: Context
    description: Additional context (conversation history, variables)

  - name: expected_output
    type: Object
    required: false
    title: Expected Output
    description: "Expected response structure or content"

  - name: evaluators
    type: Array
    required: false
    title: Evaluators
    description: "Override default evaluators for this case"

  - name: tags
    type: Array
    required: false

  - name: timeout_ms
    type: Number
    required: false
    default: 60000

  - name: enabled
    type: Boolean
    required: false
    default: true
```

**EvalRun:**
```yaml
name: raisin:EvalRun
description: Execution of an evaluation dataset
icon: play-circle
version: 1
properties:
  - name: dataset_ref
    type: Reference
    required: true
    allowed_types: [raisin:EvalDataset]

  - name: status
    type: String
    enum: [pending, running, completed, failed, cancelled]
    default: pending

  - name: started_at
    type: Date
    required: false

  - name: completed_at
    type: Date
    required: false

  - name: total_cases
    type: Number
    required: false
    readonly: true

  - name: completed_cases
    type: Number
    required: false
    readonly: true

  - name: passed_cases
    type: Number
    required: false
    readonly: true

  - name: failed_cases
    type: Number
    required: false
    readonly: true

  - name: summary
    type: Object
    required: false
    description: "Aggregated metrics across all cases"

  - name: model_override
    type: String
    required: false
    description: "Override agent's default model"

  - name: triggered_by
    type: String
    required: false

allowed_children:
  - raisin:EvalCaseResult
```

**EvalCaseResult:**
```yaml
name: raisin:EvalCaseResult
description: Result of evaluating a single case
icon: check-circle
version: 1
properties:
  - name: case_ref
    type: Reference
    required: true
    allowed_types: [raisin:EvalCase]

  - name: status
    type: String
    enum: [pending, running, passed, failed, error, skipped]
    default: pending

  - name: conversation_ref
    type: Reference
    required: false
    allowed_types: [raisin:AIConversation]
    description: "Reference to the conversation created during evaluation"

  - name: actual_output
    type: Object
    required: false
    description: "Agent's actual response"

  - name: evaluator_results
    type: Array
    required: false
    description: "Results from each evaluator"

  - name: overall_score
    type: Number
    required: false
    description: "Aggregated score 0-1"

  - name: duration_ms
    type: Number
    required: false

  - name: tokens_used
    type: Number
    required: false

  - name: cost_usd
    type: Number
    required: false

  - name: error
    type: String
    required: false
```

### Built-in Evaluators

```yaml
# Evaluator function definitions

# 1. Schema Match
name: schema_match
input_schema:
  type: object
  properties:
    schema:
      type: object
      description: JSON schema to validate against
    strict:
      type: boolean
      default: true
# Returns: { passed: bool, errors: string[] }

# 2. Contains Text
name: contains_text
input_schema:
  type: object
  properties:
    texts:
      type: array
      items:
        type: string
    mode:
      type: string
      enum: [all, any]
      default: all
    case_sensitive:
      type: boolean
      default: false
# Returns: { passed: bool, matched: string[], missing: string[] }

# 3. Semantic Similarity
name: semantic_similarity
input_schema:
  type: object
  properties:
    reference:
      type: string
      description: Text to compare against
    threshold:
      type: number
      default: 0.8
      description: Minimum similarity score (0-1)
# Returns: { passed: bool, score: number }

# 4. LLM Judge
name: llm_judge
input_schema:
  type: object
  properties:
    criteria:
      type: string
      description: Evaluation criteria for the LLM
    model:
      type: string
      default: gpt-4o
    rubric:
      type: object
      description: Scoring rubric
# Returns: { passed: bool, score: number, reasoning: string }

# 5. Tool Usage
name: tool_usage
input_schema:
  type: object
  properties:
    expected_tools:
      type: array
      items:
        type: string
    mode:
      type: string
      enum: [exact, contains, excludes]
      default: contains
# Returns: { passed: bool, actual_tools: string[], missing: string[], unexpected: string[] }

# 6. Response Time
name: response_time
input_schema:
  type: object
  properties:
    max_ms:
      type: number
      description: Maximum acceptable response time
# Returns: { passed: bool, actual_ms: number }

# 7. Custom Function
name: custom_function
input_schema:
  type: object
  properties:
    function_ref:
      type: string
      description: Reference to custom evaluator function
# Function receives: { input, expected_output, actual_output, conversation }
# Returns: { passed: bool, score?: number, details?: object }
```

### Evaluation Runner

```javascript
// eval-runner function
export async function runEvaluation(ctx) {
  const { run_id } = ctx.input;
  const run = await raisin.nodes.get(ctx.workspace, run_id);
  const dataset = await raisin.nodes.get(ctx.workspace, run.dataset_ref);
  const cases = await raisin.nodes.getChildren(ctx.workspace, dataset.path);

  // Update run status
  await raisin.nodes.update(ctx.workspace, run_id, {
    properties: {
      status: 'running',
      started_at: new Date(),
      total_cases: cases.length,
    }
  });

  const results = [];

  for (const evalCase of cases.filter(c => c.properties.enabled)) {
    const result = await evaluateCase(ctx, dataset, evalCase, run);
    results.push(result);

    // Update progress
    await raisin.nodes.update(ctx.workspace, run_id, {
      properties: {
        completed_cases: results.length,
        passed_cases: results.filter(r => r.status === 'passed').length,
        failed_cases: results.filter(r => r.status === 'failed').length,
      }
    });
  }

  // Calculate summary
  const summary = calculateSummary(results);

  await raisin.nodes.update(ctx.workspace, run_id, {
    properties: {
      status: 'completed',
      completed_at: new Date(),
      summary,
    }
  });

  return { run_id, summary };
}

async function evaluateCase(ctx, dataset, evalCase, run) {
  // 1. Create test conversation
  const conversation = await raisin.nodes.create(ctx.workspace, {
    node_type: 'raisin:AIConversation',
    parent_path: run.path,
    properties: {
      agent_ref: dataset.agent_ref,
      title: `Eval: ${evalCase.name}`,
    }
  });

  // 2. Send input message
  const startTime = Date.now();
  await raisin.nodes.create(ctx.workspace, {
    node_type: 'raisin:AIMessage',
    parent_path: conversation.path,
    properties: {
      role: 'user',
      content: evalCase.input,
    }
  });

  // 3. Wait for agent response
  const response = await waitForAgentResponse(ctx, conversation, evalCase.timeout_ms);
  const duration = Date.now() - startTime;

  // 4. Run evaluators
  const evaluators = evalCase.evaluators || dataset.default_evaluators || [];
  const evaluatorResults = [];

  for (const evaluator of evaluators) {
    const result = await runEvaluator(ctx, evaluator, {
      input: evalCase.input,
      expected_output: evalCase.expected_output,
      actual_output: response,
      conversation,
    });
    evaluatorResults.push(result);
  }

  // 5. Create result node
  const passed = evaluatorResults.every(r => r.passed);
  const overallScore = evaluatorResults.reduce((sum, r) => sum + (r.score || (r.passed ? 1 : 0)), 0) / evaluatorResults.length;

  return await raisin.nodes.create(ctx.workspace, {
    node_type: 'raisin:EvalCaseResult',
    parent_path: run.path,
    properties: {
      case_ref: evalCase.path,
      status: passed ? 'passed' : 'failed',
      conversation_ref: conversation.path,
      actual_output: response,
      evaluator_results: evaluatorResults,
      overall_score: overallScore,
      duration_ms: duration,
    }
  });
}
```

### Admin Console Integration

```
┌─────────────────────────────────────────────────────────────┐
│  Evaluation Dashboard                                        │
├─────────────────────────────────────────────────────────────┤
│                                                              │
│  📊 Dataset: Customer Support Agent Tests                    │
│  🤖 Agent: support-assistant                                 │
│                                                              │
│  ┌─────────────────────────────────────────────────────────┐│
│  │ Run History                                              ││
│  │ ─────────────────────────────────────────────────────── ││
│  │ Run #15  ✅ Passed  92%  (46/50 cases)  Dec 21, 2025    ││
│  │ Run #14  ❌ Failed  84%  (42/50 cases)  Dec 20, 2025    ││
│  │ Run #13  ✅ Passed  90%  (45/50 cases)  Dec 19, 2025    ││
│  └─────────────────────────────────────────────────────────┘│
│                                                              │
│  ┌─────────────────────────────────────────────────────────┐│
│  │ Case Results (Run #15)                         [Export] ││
│  │ ─────────────────────────────────────────────────────── ││
│  │ ✅ refund_request        1.0   245ms   $0.002           ││
│  │ ✅ product_inquiry       0.95  312ms   $0.003           ││
│  │ ❌ complex_complaint     0.65  890ms   $0.008           ││
│  │    └─ schema_match: FAILED (missing 'resolution')       ││
│  │ ✅ order_status          1.0   156ms   $0.001           ││
│  └─────────────────────────────────────────────────────────┘│
│                                                              │
│  [Run New Evaluation]  [Compare Runs]  [View Trends]        │
└─────────────────────────────────────────────────────────────┘
```

### 14. PROVIDER ARCHITECTURE IMPROVEMENTS

**Learnings from pydantic-ai's Provider System:**

pydantic-ai has a sophisticated provider architecture with 30+ providers. Key patterns RaisinDB should adopt:

#### A. ModelProfile Abstraction

**Current RaisinDB**: Scattered capability checks in provider code
**Better Approach**: Centralized profile per model

```rust
// New: raisin-ai/src/profiles.rs
#[derive(Debug, Clone)]
pub struct ModelProfile {
    pub supports_tools: bool,
    pub supports_streaming: bool,
    pub supports_native_structured_output: bool,  // Claude 3.5+, GPT-4o
    pub supports_json_mode: bool,
    pub supports_vision: bool,
    pub supports_audio: bool,
    pub default_output_mode: OutputMode,  // tool | native | prompted
    pub max_tokens: Option<u32>,
    pub context_window: u32,
    pub cost_per_1k_input: f64,
    pub cost_per_1k_output: f64,
    pub thinking_tags: Option<(String, String)>,  // ("<think>", "</think>")
    pub json_schema_transformer: Option<JsonSchemaTransformerType>,
}

// Lazy profile lookup by model name
pub fn anthropic_model_profile(model_name: &str) -> Option<ModelProfile> {
    match model_name {
        s if s.contains("claude-3-5") || s.contains("claude-3.5") => Some(ModelProfile {
            supports_tools: true,
            supports_streaming: true,
            supports_native_structured_output: true,
            context_window: 200_000,
            cost_per_1k_input: 0.003,
            cost_per_1k_output: 0.015,
            ..Default::default()
        }),
        s if s.contains("claude-opus-4") => Some(ModelProfile {
            supports_tools: true,
            supports_streaming: true,
            supports_native_structured_output: true,
            context_window: 200_000,
            thinking_tags: Some(("<thinking>".into(), "</thinking>".into())),
            cost_per_1k_input: 0.015,
            cost_per_1k_output: 0.075,
            ..Default::default()
        }),
        _ => None,
    }
}
```

#### B. Provider-Prefixed Settings

**Current RaisinDB**: Generic settings that may conflict
**Better Approach**: Namespace all provider-specific settings

```rust
// New: Provider-specific settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionSettings {
    // Common settings
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,

    // OpenAI-specific
    #[serde(skip_serializing_if = "Option::is_none")]
    pub openai_frequency_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub openai_presence_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub openai_reasoning_effort: Option<String>,  // for o1 models

    // Anthropic-specific
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anthropic_thinking_budget: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anthropic_prompt_caching: Option<bool>,

    // Google-specific
    #[serde(skip_serializing_if = "Option::is_none")]
    pub google_safety_settings: Option<Vec<SafetySetting>>,
}
```

#### C. Request Preparation Hook

**Current RaisinDB**: Each provider handles validation differently
**Better Approach**: Centralized `prepare_request()` that validates and transforms

```rust
impl AIProvider {
    pub fn prepare_request(
        &self,
        request: CompletionRequest,
        profile: &ModelProfile,
    ) -> Result<PreparedRequest, AIError> {
        // 1. Validate output mode is supported
        if request.output_mode == OutputMode::Native && !profile.supports_native_structured_output {
            return Err(AIError::UnsupportedOutputMode);
        }

        // 2. Transform JSON schema if needed
        let output_schema = if let Some(schema) = request.output_schema {
            self.transform_schema(schema, profile)?
        } else {
            None
        };

        // 3. Filter tools to supported builtin tools
        let tools = self.filter_supported_tools(request.tools, profile);

        // 4. Merge settings (request overrides model defaults)
        let settings = self.merge_settings(request.settings, profile);

        Ok(PreparedRequest { settings, tools, output_schema, .. })
    }
}
```

#### D. JSON Schema Transformation

**Problem**: Different providers have different JSON schema constraints
- Anthropic: Requires `additionalProperties: false`
- OpenAI: Supports strict mode with different rules
- Some providers don't support `$ref`

```rust
pub trait JsonSchemaTransformer {
    fn transform(&self, schema: Value) -> Value;
}

pub struct AnthropicSchemaTransformer;
impl JsonSchemaTransformer for AnthropicSchemaTransformer {
    fn transform(&self, mut schema: Value) -> Value {
        // Add additionalProperties: false to all objects
        add_additional_properties_false(&mut schema);
        // Move unsupported constraints to description
        move_constraints_to_description(&mut schema);
        schema
    }
}
```

#### E. Streaming Response Normalization

**Current RaisinDB**: Provider-specific stream handling scattered
**Better Approach**: Unified StreamedResponse with parts manager

```rust
pub struct StreamedResponse {
    pub model_name: String,
    pub provider_name: String,
    pub parts: Vec<ResponsePart>,
    pub usage: TokenUsage,
    pub finish_reason: Option<FinishReason>,
}

pub enum StreamEvent {
    ContentDelta { text: String, index: u32 },
    ToolCallStart { id: String, name: String },
    ToolCallArguments { id: String, delta: String },
    ThinkingDelta { text: String },
    Done { finish_reason: FinishReason, usage: TokenUsage },
}

// Each provider converts its stream format to StreamEvent
pub trait StreamingProvider {
    fn stream_to_events(
        vendor_stream: impl Stream<Item = VendorChunk>,
    ) -> impl Stream<Item = StreamEvent>;
}
```

#### F. Gateway Pattern for Provider Abstraction

**Enables**: Switching providers without code changes

```rust
// URL-based provider routing
// "gateway/openai:gpt-4o" → routes through gateway to OpenAI
// "gateway/anthropic:claude-opus-4" → routes through gateway to Anthropic

pub struct GatewayProvider {
    gateway_url: String,
    upstream_provider: String,
}

impl GatewayProvider {
    pub async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse> {
        // Add tracing headers
        // Route to upstream via gateway
        // Gateway handles auth, logging, rate limiting
    }
}
```

#### G. Error Hierarchy Improvement

**Current RaisinDB**: Generic AIError
**Better Approach**: Specific exception types

```rust
#[derive(Debug, Error)]
pub enum AIError {
    #[error("HTTP error {status}: {message}")]
    HttpError { status: u16, message: String, body: Option<String> },

    #[error("API error from {provider}: {message}")]
    ApiError { provider: String, message: String },

    #[error("Unexpected model behavior: {message}")]
    UnexpectedBehavior { message: String, response: Option<String> },

    #[error("Usage limit exceeded: {limit_type}")]
    UsageLimitExceeded { limit_type: String },

    #[error("Model retry requested: {reason}")]
    ModelRetry { reason: String },  // For self-correction

    #[error("Output validation failed: {errors:?}")]
    OutputValidationError { errors: Vec<String> },

    #[error("Unsupported feature: {feature} for model {model}")]
    UnsupportedFeature { feature: String, model: String },
}
```

#### H. Client Caching & Connection Pooling

```rust
// Cached per-provider HTTP clients
lazy_static! {
    static ref HTTP_CLIENTS: DashMap<String, Arc<reqwest::Client>> = DashMap::new();
}

pub fn get_http_client(provider: &str) -> Arc<reqwest::Client> {
    HTTP_CLIENTS.entry(provider.to_string()).or_insert_with(|| {
        Arc::new(
            reqwest::Client::builder()
                .timeout(Duration::from_secs(600))
                .connect_timeout(Duration::from_secs(5))
                .user_agent(format!("raisindb/{}", env!("CARGO_PKG_VERSION")))
                .build()
                .unwrap()
        )
    }).clone()
}
```

#### I. Implementation Roadmap

**Phase 1: ModelProfile (1 week)**
- Create `profiles.rs` with ModelProfile struct
- Add profile functions for each provider
- Wire profiles into existing providers

**Phase 2: Settings Namespacing (3 days)**
- Refactor CompletionSettings with provider prefixes
- Update all providers to use new settings
- Maintain backwards compatibility

**Phase 3: Request Preparation (1 week)**
- Create unified `prepare_request()` hook
- Add JSON schema transformers per provider
- Centralize validation logic

**Phase 4: Streaming Normalization (1 week)**
- Create StreamEvent enum
- Implement stream conversion for each provider
- Wire into existing event system

**Phase 5: Error Hierarchy (3 days)**
- Create specific error types
- Update all providers to use new errors
- Add error context (status codes, response bodies)

---

### 15. OBSERVABILITY (OpenTelemetry)

**Enhancement Ideas:**

**A. Span per Agent Invocation**
```
agent_run
├── build_history (duration, message_count)
├── resolve_tools (tool_count)
├── ai_completion
│   ├── provider: openai
│   ├── model: gpt-4
│   ├── input_tokens: 1234
│   └── output_tokens: 567
├── tool_execution[]
│   ├── tool_name: weather
│   └── duration_ms: 150
└── save_response
```

**B. Metrics**
- `ai.completion.duration` - Histogram
- `ai.completion.tokens` - Counter
- `ai.tool.execution.duration` - Histogram
- `agent.iteration.count` - Counter
- `agent.error.count` - Counter by error type

**C. Cost Tracking**
- Track cost per completion (model pricing)
- Aggregate by agent, conversation, tenant
- Dashboard in admin-console

---

### 16. PROMPT FORMATTING PATTERNS

**Learnings from pydantic-ai's Prompt Assembly:**

pydantic-ai uses a structured approach to build prompts with clear separation of concerns. Key patterns RaisinDB should adopt.

---

## PYDANTIC-AI PROMPTS & TEMPLATES REFERENCE

> **Source:** Extracted from pydantic-ai codebase for RaisinDB compatibility

### Core Prompts

#### 1. DEFAULT PROMPTED OUTPUT TEMPLATE
**Source:** `pydantic_ai_slim/pydantic_ai/profiles/__init__.py:44-52`
**Used For:** When model supports text output but not native JSON schema

```python
prompted_output_template: str = dedent(
    """
    Always respond with a JSON object that's compatible with this schema:

    {schema}

    Don't include any text or Markdown fencing before or after.
    """
)
```

#### 2. MISTRAL JSON MODE SCHEMA PROMPT
**Source:** `pydantic_ai_slim/pydantic_ai/models/mistral.py:143`
**Used For:** Mistral model's JSON mode output formatting

```python
json_mode_schema_prompt: str = """Answer in JSON Object, respect the format:\n```\n{schema}\n```\n"""
```

#### 3. OUTPUT TOOL DEFAULTS
**Source:** `pydantic_ai_slim/pydantic_ai/_output.py:72-73`
**Used For:** Tool-based output mode naming

```python
DEFAULT_OUTPUT_TOOL_NAME = 'final_result'
DEFAULT_OUTPUT_TOOL_DESCRIPTION = 'The final response which ends this conversation'
```

#### 4. RETRY/VALIDATION ERROR PROMPTS
**Source:** `pydantic_ai_slim/pydantic_ai/messages.py:908-960`
**Used For:** Self-correction when validation fails

```python
# Pattern 1: Simple string error (no tool context)
f'Validation feedback:\n{error_text}'

# Pattern 2: Tool-specific error
f'{error_text}'  # Direct error message

# Pattern 3: Structured validation errors
f'{count} validation error{"s" if plural else ""}:\n```json\n{json_errors}\n```\n\nFix the errors and try again.'
```

#### 5. UNKNOWN TOOL ERROR MESSAGE
**Source:** `pydantic_ai_slim/pydantic_ai/_tool_manager.py:144-149`
**Used For:** When model calls non-existent tool

```python
# When tools exist:
f'Unknown tool name: {name!r}. Available tools: {", ".join(f"{name!r}" for name in self.tools.keys())}'

# When no tools:
f'Unknown tool name: {name!r}. No tools available.'
```

---

### Provider-Specific Formats

#### 6. OPENAI JSON SCHEMA RESPONSE FORMAT
**Source:** `pydantic_ai_slim/pydantic_ai/models/openai.py`
**Used For:** Native structured output (GPT-4o, etc.)

```python
response_format_param = {
    'type': 'json_schema',
    'json_schema': {
        'name': output_name or 'final_result',
        'schema': json_schema,
        'strict': True,  # or False
    }
}
```

#### 7. ANTHROPIC NATIVE OUTPUT FORMAT
**Source:** `pydantic_ai_slim/pydantic_ai/models/anthropic.py:1131`
**Used For:** Claude structured output (Claude 3.5+)

```python
response_format = {
    'type': 'json',
    'json_schema': json_schema
}
```

#### 8. GOOGLE/GEMINI SYSTEM INSTRUCTION
**Source:** `pydantic_ai_slim/pydantic_ai/models/google.py:669-671`
**Used For:** Gemini system prompts (role must be 'user')

```python
# Gemini wraps system instructions as user content
system_instruction = {
    'role': 'user',
    'parts': [{'text': instructions}]
}
```

#### 9. GROQ JSON RESPONSE FORMAT
**Source:** `pydantic_ai_slim/pydantic_ai/models/groq.py:280-286`
**Used For:** Groq structured output

```python
# With schema:
response_format = {
    'type': 'json_schema',
    'json_schema': {
        'name': output_name,
        'schema': json_schema,
        'strict': True
    }
}

# Without schema (json mode only):
response_format = {'type': 'json_object'}
```

---

### System Prompt Role Configuration

#### 10. OPENAI SYSTEM PROMPT ROLES
**Source:** `pydantic_ai_slim/pydantic_ai/profiles/openai.py:60-61`
**Used For:** Different OpenAI models require different roles

```python
# Role options:
openai_system_prompt_role = 'system'    # Default for most models
openai_system_prompt_role = 'developer' # For certain newer models
openai_system_prompt_role = 'user'      # For o1-mini (no system role support)
```

---

### Thinking/Reasoning Tags

#### 11. THINKING CONTENT DELIMITERS
**Source:** `pydantic_ai_slim/pydantic_ai/profiles/__init__.py:59`
**Used For:** Extended thinking models (o-series, Claude with thinking)

```python
thinking_tags: tuple[str, str] = ('<think>', '</think>')

# Alternative for some models:
thinking_tags = ('<thinking>', '</thinking>')
```

---

### JSON Schema Transformers

#### 12. OPENAI STRICT MODE TRANSFORMER
**Source:** `pydantic_ai_slim/pydantic_ai/profiles/openai.py:139-274`
**Used For:** Making schemas compatible with OpenAI strict mode

```python
# Rules applied:
# 1. Remove unsupported keys
REMOVE_KEYS = {'title', '$schema', 'discriminator'}
STRICT_MODE_REMOVE_KEYS = {
    'minLength', 'maxLength', 'pattern', 'minimum', 'maximum',
    'exclusiveMinimum', 'exclusiveMaximum', 'multipleOf',
    'minItems', 'maxItems', 'uniqueItems', 'minProperties',
    'maxProperties', 'patternProperties', 'propertyNames',
    'unevaluatedItems', 'unevaluatedProperties', 'contentEncoding',
    'contentMediaType', 'contentSchema', 'prefixItems', 'if',
    'then', 'else', 'minContains', 'maxContains'
}

# 2. Set additionalProperties: false on all objects
def transform_object(schema):
    schema['additionalProperties'] = False
    # Make all properties required
    if 'properties' in schema:
        schema['required'] = list(schema['properties'].keys())
    return schema
```

#### 13. ANTHROPIC SCHEMA TRANSFORMER
**Used For:** Making schemas compatible with Anthropic

```python
# Rules applied:
# 1. additionalProperties: false required on all objects
# 2. Move unsupported constraints to description field
# 3. $ref resolution may be required
```

---

### Mistral Python Type Mapping

#### 14. JSON SCHEMA TO PYTHON TYPE HINTS
**Source:** `pydantic_ai_slim/pydantic_ai/models/mistral.py`
**Used For:** Human-readable type hints in Mistral prompts

```python
TYPE_MAPPING = {
    'string': 'str',
    'integer': 'int',
    'number': 'float',
    'boolean': 'bool',
    'array': 'list[...]',
    'object': 'dict[str, ...]',
    'null': 'None',
}

# Example output in prompt:
# "Answer in JSON Object, respect the format:
# ```
# {"name": str, "age": int, "active": bool}
# ```"
```

---

### XML Formatting Utility

#### 15. PYTHON OBJECT TO XML
**Source:** `pydantic_ai_slim/pydantic_ai/format_prompt.py:17-74`
**Used For:** Structured data in prompts (Claude prefers XML)

```python
def format_as_xml(
    data: Any,
    root_tag: str = 'data',
    indent: int = 2,
) -> str:
    """Convert Python objects to XML string.

    Example:
        >>> format_as_xml({'name': 'John', 'age': 30}, root_tag='user')
        '<user>
          <name>John</name>
          <age>30</age>
        </user>'
    """
```

---

### Dynamic Instruction Builder

#### 16. OUTPUT INSTRUCTIONS BUILDER
**Source:** `pydantic_ai_slim/pydantic_ai/_output.py:462-474`
**Used For:** Generating complete output instructions

```python
@classmethod
def build_instructions(cls, template: str, object_def: OutputObjectDefinition) -> str:
    """Build instructions from a template and an object definition."""
    schema = object_def.json_schema.copy()
    if object_def.name:
        schema['title'] = object_def.name
    if object_def.description:
        schema['description'] = object_def.description

    if '{schema}' not in template:
        template = '\n\n'.join([template, '{schema}'])

    return template.format(schema=json.dumps(schema))
```

---

### Finish Reason Constants

#### 17. NORMALIZED FINISH REASONS
**Source:** `pydantic_ai_slim/pydantic_ai/messages.py:58-64`
**Used For:** Consistent finish reasons across providers

```python
FinishReason = Literal[
    'stop',           # Normal completion
    'length',         # Max tokens reached
    'content_filter', # Content filtered by provider
    'tool_call',      # Model made tool call(s)
    'error',          # Error occurred
]
```

---

## SUMMARY: PROMPTS TO IMPLEMENT IN RAISINDB

| Prompt Type | Priority | Implementation Location |
|-------------|----------|------------------------|
| Prompted Output Template | High | `raisin-ai/profiles.rs` |
| Output Tool Defaults | High | `raisin-ai/output.rs` |
| Retry Error Messages | High | `agent-handler` |
| Unknown Tool Message | Medium | `tool-executor` |
| OpenAI JSON Schema Format | High | `openai_provider.rs` |
| Anthropic JSON Format | High | `anthropic_provider.rs` |
| Gemini System Instruction | High | `google_provider.rs` |
| Thinking Tags | Medium | `profiles.rs` |
| Schema Transformers | High | `schema_transform.rs` |
| XML Formatter | Low | Utility function |
| Finish Reasons | High | `messages.rs` |

---

## RAISINDB IMPLEMENTATION GUIDE

> Implementation examples for integrating the above prompts into RaisinDB

#### A. Message Structure (JavaScript/TypeScript)

```javascript
// Message part types (inspired by pydantic-ai)
const MessagePart = {
  SystemPrompt: { content: string },
  UserPrompt: { content: string, timestamp?: Date },
  TextPart: { content: string },
  ToolReturn: { tool_name: string, content: any, tool_call_id: string },
  RetryPrompt: { content: string },  // For self-correction
};

// Full message structure
const ModelRequest = {
  parts: MessagePart[],
  kind: 'request'
};

const ModelResponse = {
  parts: ResponsePart[],  // TextPart | ToolCallPart
  kind: 'response',
  usage: { input_tokens, output_tokens },
};
```

#### B. Prompted Output Templates (JavaScript Constants)

```javascript
// Default template for prompted output mode
const DEFAULT_OUTPUT_TEMPLATE = `
Always respond with a JSON object that matches this schema:

{schema}

Respond only with the JSON object, no other text.
Don't include markdown code blocks, just the raw JSON.
`;

// Alternative: More detailed template with examples
const DETAILED_OUTPUT_TEMPLATE = `
You must respond with a valid JSON object matching this exact schema:

{schema}

Requirements:
- Output ONLY the JSON, no explanations or markdown
- All required fields must be present
- Use null for optional missing values
- Strings must be properly escaped

Example format:
{example}
`;
```

#### C. Tool Definition Format (RaisinDB Function → AI Tool)

```javascript
// Convert RaisinDB function to AI tool format
function functionToToolDefinition(func) {
  return {
    type: 'function',
    function: {
      name: func.properties.name,
      description: func.properties.description || '',
      parameters: func.properties.input_schema || {
        type: 'object',
        properties: {},
      },
      // Provider-specific extensions
      strict: true,  // OpenAI strict mode
    },
  };
}
```

#### D. XML Formatting Helper (Utility Function)

```javascript
// XML formatting for complex instructions
function formatXml(tagName, content, attrs = {}) {
  const attrStr = Object.entries(attrs)
    .map(([k, v]) => `${k}="${v}"`)
    .join(' ');
  const openTag = attrStr ? `<${tagName} ${attrStr}>` : `<${tagName}>`;
  return `${openTag}\n${content}\n</${tagName}>`;
}

// Usage
const systemPrompt = `
${formatXml('role', 'You are a helpful assistant')}

${formatXml('rules', `
- Be concise
- Use structured output
- Cite sources
`)}

${formatXml('output_format', JSON.stringify(outputSchema, null, 2))}
`;
```

#### E. Complete Request Assembly (Agent Handler)

```javascript
// In agent handler - builds messages for AI completion
function buildMessages(agent, conversation, outputConfig) {
  const messages = [];

  // 1. System prompt (always first)
  if (agent.properties.system_prompt) {
    messages.push({
      role: 'system',
      content: agent.properties.system_prompt
    });
  }

  // 2. Additional instructions (with output formatting)
  let instructions = agent.properties.instructions || '';

  if (outputConfig?.mode === 'prompted' && outputConfig?.schema) {
    const template = outputConfig.template || DEFAULT_OUTPUT_TEMPLATE;
    const schemaInstructions = template
      .replace('{schema}', JSON.stringify(outputConfig.schema, null, 2))
      .replace('{example}', outputConfig.example || '');

    instructions = instructions
      ? `${instructions}\n\n${schemaInstructions}`
      : schemaInstructions;
  }

  if (instructions) {
    messages.push({
      role: 'system',
      content: instructions
    });
  }

  // 3. Conversation history
  for (const msg of conversation.messages) {
    if (msg.role === 'user') {
      messages.push({ role: 'user', content: msg.content });
    } else if (msg.role === 'assistant') {
      messages.push({ role: 'assistant', content: msg.content });
    } else if (msg.role === 'tool') {
      messages.push({
        role: 'tool',
        tool_call_id: msg.tool_call_id,
        content: JSON.stringify(msg.result),
      });
    }
  }

  return messages;
}
```

#### F. AIAgent Schema Extensions

Add these properties to `raisin:AIAgent`:

```yaml
# In ai_agent.yaml
properties:
  # ... existing properties ...

  - name: instructions
    type: String
    required: false
    title: Additional Instructions
    description: Additional instructions appended after system prompt

  - name: output_config
    type: Object
    required: false
    title: Output Configuration
    description: |
      Controls how structured output is requested from the model.
      Structure: {
        mode: 'native' | 'prompted' | 'tool',
        schema: JSON Schema,
        template: optional custom template,
        example: optional example output
      }
```

Usage in agent handler (Rust):

```rust
// In raisin-ai agent handler
fn build_completion_request(
    agent: &AIAgent,
    conversation: &AIConversation,
) -> CompletionRequest {
    let mut messages = Vec::new();

    // 1. System prompt
    if let Some(system) = &agent.system_prompt {
        messages.push(Message::system(system.clone()));
    }

    // 2. Instructions with output formatting
    let mut instructions = agent.instructions.clone().unwrap_or_default();

    if let Some(output_config) = &agent.output_config {
        if output_config.mode == OutputMode::Prompted {
            if let Some(schema) = &output_config.schema {
                let template = output_config.template.as_deref()
                    .unwrap_or(DEFAULT_OUTPUT_TEMPLATE);
                let schema_str = serde_json::to_string_pretty(schema)?;
                let formatted = template.replace("{schema}", &schema_str);

                if instructions.is_empty() {
                    instructions = formatted;
                } else {
                    instructions.push_str("\n\n");
                    instructions.push_str(&formatted);
                }
            }
        }
    }

    if !instructions.is_empty() {
        messages.push(Message::system(instructions));
    }

    // 3. Conversation history
    for msg in &conversation.messages {
        messages.push(msg.to_api_message());
    }

    CompletionRequest {
        messages,
        tools: resolve_tools(agent)?,
        output_schema: if output_config.mode == OutputMode::Native {
            output_config.schema.clone()
        } else {
            None
        },
        ..Default::default()
    }
}
```

#### G. Provider-Specific Message Formatting (Rust)

Each provider has different requirements for message formatting:

```rust
// Provider-specific prompt adjustments
impl PromptFormatter {
    fn format_for_provider(&self, provider: &str, messages: Vec<Message>) -> Vec<Message> {
        match provider {
            "anthropic" => {
                // Claude prefers XML tags for structure
                // Merge consecutive system messages
                self.merge_system_messages(messages)
            }
            "openai" => {
                // GPT handles multiple system messages well
                // Use JSON for structured instructions
                messages
            }
            "google" => {
                // Gemini: first message must be user, not system
                self.convert_leading_system_to_context(messages)
            }
            _ => messages
        }
    }
}
```

---

### 17. PROVIDER COVERAGE COMPARISON

#### Current RaisinDB Providers (5)

| Provider | File | Streaming | Tools | Embeddings | Vision |
|----------|------|-----------|-------|------------|--------|
| **OpenAI** | `openai.rs` | ✅ | ✅ | ✅ | ✅ |
| **Anthropic** | `anthropic.rs` | ✅ | ✅ | ❌ | ✅ |
| **Google Gemini** | `gemini.rs` | ✅ | ✅ | ❌ | ✅ |
| **Ollama** | `ollama.rs` | ✅ | ✅ | ✅ | ✅ |
| **Azure OpenAI** | `azure_openai.rs` | ✅ | ✅ | ✅ | ✅ |

#### Pydantic-AI Providers (31) - Full List

**Tier 1: Major Cloud Providers** (RaisinDB has these)
| Provider | pydantic-ai | RaisinDB | Priority |
|----------|-------------|----------|----------|
| OpenAI | ✅ | ✅ | - |
| Anthropic | ✅ | ✅ | - |
| Google (Gemini) | ✅ | ✅ | - |
| Azure OpenAI | ✅ | ✅ | - |
| Ollama (local) | ✅ | ✅ | - |

**Tier 2: Cloud Platforms** (High Priority to Add)
| Provider | pydantic-ai | RaisinDB | Priority | Notes |
|----------|-------------|----------|----------|-------|
| **AWS Bedrock** | ✅ | ❌ | **HIGH** | Access to Claude, Llama, Cohere, Mistral on AWS |
| **Google Vertex AI** | ✅ | ❌ | **HIGH** | Enterprise Gemini with VPC |
| **Groq** | ✅ | ❌ | **HIGH** | Ultra-fast inference, Llama models |
| **Mistral** | ✅ | ❌ | **MEDIUM** | European AI, good for EU compliance |

**Tier 3: Aggregators/Proxies** (Medium Priority)
| Provider | pydantic-ai | RaisinDB | Priority | Notes |
|----------|-------------|----------|----------|-------|
| **OpenRouter** | ✅ | ❌ | **MEDIUM** | Access 100+ models via single API |
| **LiteLLM** | ✅ | ❌ | **MEDIUM** | Universal proxy for any provider |
| **Together AI** | ✅ | ❌ | **MEDIUM** | Open source model hosting |
| **Fireworks AI** | ✅ | ❌ | **LOW** | Fast inference platform |

**Tier 4: Specialized Providers** (Lower Priority)
| Provider | pydantic-ai | RaisinDB | Priority | Notes |
|----------|-------------|----------|----------|-------|
| **DeepSeek** | ✅ | ❌ | **MEDIUM** | Cost-effective, good reasoning |
| **Cohere** | ✅ | ❌ | **LOW** | RAG-focused, Command models |
| **HuggingFace** | ✅ | ❌ | **LOW** | Inference API, open models |
| **Cerebras** | ✅ | ❌ | **LOW** | Ultra-fast wafer-scale inference |
| **xAI Grok** | ✅ | ❌ | **LOW** | Twitter/X integration |

**Tier 5: Regional/Niche Providers** (Low Priority)
| Provider | pydantic-ai | RaisinDB | Priority | Notes |
|----------|-------------|----------|----------|-------|
| **Alibaba (Qwen)** | ✅ | ❌ | **LOW** | Chinese market, Qwen models |
| **Moonshot AI (Kimi)** | ✅ | ❌ | **LOW** | Chinese market |
| **Nebius** | ✅ | ❌ | **LOW** | Russian/EU market |
| **OVHcloud** | ✅ | ❌ | **LOW** | European hosting |
| **Heroku AI** | ✅ | ❌ | **LOW** | Heroku platform integration |
| **Vercel AI** | ✅ | ❌ | **LOW** | Vercel platform integration |
| **GitHub Models** | ✅ | ❌ | **LOW** | GitHub Copilot backend |

**Tier 6: Utility/Testing**
| Provider | pydantic-ai | RaisinDB | Priority | Notes |
|----------|-------------|----------|----------|-------|
| **Function Model** | ✅ | ❌ | **MEDIUM** | Custom/testing |
| **Test Model** | ✅ | ❌ | **MEDIUM** | Deterministic testing |
| **Fallback Model** | ✅ | ❌ | **HIGH** | Multi-model fallback |
| **Instrumented** | ✅ | ❌ | **MEDIUM** | OpenTelemetry |
| **Outlines** | ✅ | ❌ | **LOW** | Guided generation |

---

#### Provider Implementation Roadmap

**Phase 1: Enterprise Cloud (High Value)**
```
1. AWS Bedrock      - Access Claude/Llama/Mistral on AWS infrastructure
2. Google Vertex AI - Enterprise Gemini with VPC/compliance
3. Groq             - Ultra-fast inference for latency-sensitive apps
```

**Phase 2: Aggregators (Flexibility)**
```
4. OpenRouter    - Single API for 100+ models
5. LiteLLM       - Universal proxy (self-hosted option)
6. Together AI   - Open source model hosting
```

**Phase 3: Specialized (Use-Case Specific)**
```
7. Mistral       - EU compliance, European hosting
8. DeepSeek      - Cost-effective reasoning
9. Cohere        - RAG-optimized models
```

**Phase 4: Utility Models**
```
10. Fallback Model    - Automatic failover
11. Test Model        - Unit testing
12. Instrumented      - Observability
```

---

#### Detailed Provider Specifications

##### AWS Bedrock Provider
```rust
// Proposed: crates/raisin-ai/src/providers/bedrock.rs

pub struct BedrockProvider {
    client: BedrockRuntimeClient,
    region: String,
}

// Supported model families:
// - anthropic.claude-* (Claude 3.5, 4, 4.5)
// - amazon.nova-* (Nova Pro, Lite, Micro)
// - meta.llama* (Llama 3.1, 3.2, 3.3)
// - cohere.command-* (Command R, R+)
// - mistral.* (Mistral Large, Small)

// Features:
// - AWS IAM authentication
// - Regional endpoints (us-east-1, eu-west-1, etc.)
// - Guardrails integration
// - Cross-region inference
// - Provisioned throughput
```

##### Groq Provider
```rust
// Proposed: crates/raisin-ai/src/providers/groq.rs

pub struct GroqProvider {
    client: reqwest::Client,
    api_key: String,
}

// Supported models:
// - llama-3.3-70b-versatile
// - llama-3.1-8b-instant
// - mixtral-8x7b-32768
// - gemma2-9b-it

// Features:
// - Ultra-fast inference (100+ tokens/sec)
// - Tool calling support
// - Streaming
// - Web search builtin tool
```

##### OpenRouter Provider
```rust
// Proposed: crates/raisin-ai/src/providers/openrouter.rs

pub struct OpenRouterProvider {
    client: reqwest::Client,
    api_key: String,
}

// Access to 100+ models including:
// - openai/* (GPT-4, GPT-4o)
// - anthropic/* (Claude 3.5, 4)
// - google/* (Gemini)
// - meta-llama/* (Llama 3)
// - mistralai/* (Mistral)
// - deepseek/* (DeepSeek)
// - And many more...

// Features:
// - Unified API for all providers
// - Automatic fallback
// - Cost tracking
// - Provider routing
```

##### Fallback Model (Utility)
```rust
// Proposed: crates/raisin-ai/src/providers/fallback.rs

pub struct FallbackProvider {
    providers: Vec<Box<dyn AIProviderTrait>>,
    strategy: FallbackStrategy,
}

pub enum FallbackStrategy {
    Sequential,      // Try in order
    RoundRobin,      // Distribute load
    LowestLatency,   // Pick fastest
    LowestCost,      // Pick cheapest
}

// Usage:
// let fallback = FallbackProvider::new(vec![
//     Box::new(OpenAIProvider::new()?),
//     Box::new(AnthropicProvider::new()?),
//     Box::new(GroqProvider::new()?),
// ]);
```

---

#### Provider Feature Matrix (Complete)

| Feature | OpenAI | Anthropic | Google | Groq | Bedrock | Mistral | OpenRouter |
|---------|--------|-----------|--------|------|---------|---------|------------|
| **Streaming** | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **Tools** | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **Native JSON** | ✅ | ✅ | ✅ | ✅ | ✅ | ❌ | Varies |
| **Vision** | ✅ | ✅ | ✅ | ❌ | ✅ | ✅ | Varies |
| **Audio** | ✅ | ❌ | ❌ | ✅ | ✅ | ❌ | Varies |
| **Embeddings** | ✅ | ❌ | ✅ | ❌ | ✅ | ✅ | Varies |
| **Thinking** | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | Varies |
| **Web Search** | ✅ | ✅ | ✅ | ✅ | ✅ | ❌ | Varies |
| **Code Exec** | ✅ | ✅ | ✅ | ❌ | ✅ | ❌ | Varies |
| **Caching** | ✅ | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ |

---

#### Model Profiles per Provider (from pydantic-ai)

##### OpenAI Profile
```python
# Key settings
openai_system_prompt_role: 'system' | 'developer' | 'user'  # Model-dependent
openai_allow_multiple_sampling_settings: bool
openai_force_response_schema_first: bool
openai_web_search_pattern: str  # Regex for web search models

# Thinking configuration (o-series)
thinking_tags: ('<think>', '</think>')

# JSON schema transformer rules:
REMOVE_KEYS = {'title', '$schema', 'discriminator'}
STRICT_MODE_REMOVE_KEYS = {
    'minLength', 'maxLength', 'pattern', 'minimum', 'maximum',
    'exclusiveMinimum', 'exclusiveMaximum', 'multipleOf',
    'minItems', 'maxItems', 'uniqueItems', ...
}
```

##### Anthropic Profile
```python
# Thinking tags
thinking_tags: ('<thinking>', '</thinking>')

# Schema requirements
# - additionalProperties: false on all objects
# - All properties must be required

# Cache control headers supported
```

##### Google Profile
```python
# System instruction as user role
# Native JSON schema for Gemini 3+
# Image output for specific models

# Safety settings support
safety_settings: List[SafetySetting]
```

##### Groq Profile
```python
# Web search builtin tool detection
groq_web_search_pattern: str

# Whitespace stripping in streams
```

##### Bedrock Profile
```python
# Regional model variants
# - us.anthropic.claude-*
# - eu.anthropic.claude-*

# Guardrails configuration
guardrail_identifier: str
guardrail_version: str

# Performance configuration
performance_config: PerformanceConfiguration
```

---

## UPDATED IMPLEMENTATION PRIORITY MATRIX

| Feature | User Value | Complexity | Dependencies |
|---------|-----------|------------|--------------|
| **Provider Architecture** | High | Medium | profiles.rs, settings refactor |
| **Streaming** | High | Medium | Rust streaming, event system |
| **Output validation** | High | Low | Add jsonschema-rs crate |
| **ModelRetry pattern** | High | Medium | Agent handler changes |
| **Multi-model fallbacks** | High | Low-Med | Step config or flow-level |
| **MCP integration** | High | High | New crate, connection pool |
| **Eval framework** | High | High | New node types, evaluators, UI |
| **Memory/RAG** | High | Medium | Vector search, history injection |
| **Cost optimization** | Medium-High | Medium | Token tracking, budgets, caching |
| **A/B testing** | Medium | Medium-High | Experiment node types, metrics |
| **Guardrails** | Medium | Medium | Pre/post hooks |
| **Multi-agent handoffs** | Medium | Medium | Agent-as-tool or flow routing |
| **Dynamic tools** | Medium | Medium | Agent handler changes |
| **History processors** | Medium | Medium | Agent handler changes |
| **OpenTelemetry** | Medium | Medium | Add tracing crate |

## QUICK WINS (Implement in days)
1. **jsonschema-rs** - Output validation (~1 day)
2. **Fallbacks in step config** - Add `model_fallbacks` property
3. **Cost tracking** - Add `raisin:AICostRecord` node type
4. **Basic guardrails** - Input/output hooks via existing functions
5. **ModelProfile struct** - Centralize model capabilities (~2 days)

## MEDIUM EFFORT (Implement in 1-2 weeks)
1. **Provider Architecture** - profiles.rs + settings refactor + request prep hook
2. **Streaming** - Needs Rust changes but builds on existing event infra
3. **ModelRetry** - Agent handler changes + new error handling
4. **History processors** - Functions for context management
5. **Memory/RAG** - Uses existing vector search, adds memory node type

## STRATEGIC INVESTMENTS (2-4 weeks)
1. **MCP integration** - New crate, protocol impl, connection pool
2. **Eval framework** - Node types, runner, evaluators, UI
3. **A/B testing** - Experiment infrastructure, metrics collection

## UNIQUE DIFFERENTIATORS (vs pydantic-ai)

| RaisinDB Advantage | Description |
|-------------------|-------------|
| **Flow runtime** | Long-running workflows with saga compensation |
| **Node persistence** | Full audit trail, versioning, branching |
| **HumanTask in flows** | Native human-in-the-loop at workflow level |
| **Visual flow editor** | If present in admin-console |
| **Unified platform** | DB + Functions + AI in one |
| **Tenant isolation** | Multi-tenant by design |

**pydantic-ai strengths RaisinDB should adopt:**
- Type-safe RunContext for dependency injection
- Rich streaming with validation
- Built-in evaluators and datasets
- Model fallback chains
