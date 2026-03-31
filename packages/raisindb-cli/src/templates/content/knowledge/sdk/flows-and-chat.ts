export function sdkFlowsAndChatKnowledge(): string {
  return `# SDK: Flows and Chat

## Flows

RaisinDB flows are server-side workflows. The SDK provides two clients:
- **FlowsApi** (WebSocket): Real-time flow operations through the existing WS connection
- **FlowClient** (HTTP/SSE): Standalone HTTP-based flow client with SSE event streaming

### FlowsApi (WebSocket)

Access through the database:

\`\`\`typescript
const db = client.database('my-repo');
const flows = db.flows();

// Run a flow
const { instance_id, job_id } = await flows.run('/flows/process-order', { orderId: '123' });

// Get instance status
const status = await flows.getInstanceStatus(instance_id);
// status.status: 'running' | 'waiting' | 'completed' | 'failed'

// Resume a waiting flow
await flows.resume(instance_id, { approved: true });

// Cancel a running flow
await flows.cancel(instance_id);

// Subscribe to events
const sub = await flows.subscribeEvents(instance_id);
for await (const event of sub.events) {
  console.log(event.type, event);
  if (event.type === 'flow_completed' || event.type === 'flow_failed') break;
}
await sub.unsubscribe();
\`\`\`

### FlowClient (HTTP/SSE)

For standalone usage or server-side environments:

\`\`\`typescript
import { FlowClient } from '@raisindb/client';

const flows = new FlowClient('http://localhost:8081', 'my-repo', authManager);

// Run and stream events
const { instance_id } = await flows.run('/flows/my-flow', { key: 'value' });
for await (const event of flows.streamEvents(instance_id)) {
  if (event.type === 'text_chunk') process.stdout.write(event.text);
  if (event.type === 'flow_completed') console.log('Done:', event.output);
}

// Run and wait for final result
const result = await flows.runAndWait('/flows/process', { input: 'data' });
if (result.status === 'completed') console.log(result.output);

// Respond to human tasks
await flows.respondToHumanTask(instance_id, 'step-5', { action: 'approve' });
\`\`\`

### Flow Event Types

- \`step_started\` / \`step_completed\` / \`step_failed\` - Step lifecycle
- \`flow_waiting\` / \`flow_resumed\` - Flow pause/resume
- \`flow_completed\` / \`flow_failed\` - Terminal events
- \`text_chunk\` / \`thought_chunk\` - AI streaming tokens
- \`tool_call_started\` / \`tool_call_completed\` - AI tool use
- \`conversation_created\` / \`message_saved\` - Chat persistence
- \`log\` - Runtime log messages

## Chat

ChatClient provides a high-level conversational AI interface built on flows.

\`\`\`typescript
import { ChatClient } from '@raisindb/client';

const chat = new ChatClient('http://localhost:8081', 'my-repo', authManager);
\`\`\`

### Create a Conversation

\`\`\`typescript
const convo = await chat.createConversation({
  agent: '/agents/support',
  input: { context: 'billing' },
});
// convo.instanceId - flow instance ID
// convo.initialEvents - initial ChatEvent[] (e.g., assistant greeting)
\`\`\`

### Send Messages and Stream Responses

\`\`\`typescript
for await (const event of chat.sendMessage(convo.instanceId, 'Hello!')) {
  switch (event.type) {
    case 'text_chunk':       process.stdout.write(event.text); break;
    case 'thought_chunk':    /* AI thinking */ break;
    case 'tool_call_started': console.log('Calling:', event.functionName); break;
    case 'tool_call_completed': console.log('Result:', event.result); break;
    case 'waiting':          console.log('Ready for next message'); break;
    case 'completed':        console.log('Session ended'); break;
    case 'failed':           console.error('Error:', event.error); break;
  }
}
\`\`\`

### One-Shot Chat

\`\`\`typescript
const { response, conversationId } = await chat.chat('/agents/support', 'What are your hours?');
console.log(response);
\`\`\`

### Message History

\`\`\`typescript
const messages = await chat.getMessages(convo.instanceId);
for (const msg of messages) {
  console.log(msg.role, msg.content);
}
\`\`\`

### Resume a Conversation

Reconnect to an existing conversation (e.g., after page reload):

\`\`\`typescript
const convo = await chat.resumeConversation(previousInstanceId);
if (convo) {
  // Continue sending messages
}
\`\`\`

### Framework Integrations

The SDK includes React and Svelte adapters:

\`\`\`typescript
// React
import { useChat } from '@raisindb/client';
const { messages, sendMessage, isStreaming } = useChat(chatClient, { agent: '/agents/support' });

// Svelte
import { ChatStore } from '@raisindb/client';
const store = new ChatStore(chatClient, { agent: '/agents/support' });
\`\`\`
`;
}
