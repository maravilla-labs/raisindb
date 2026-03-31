# Launchpad Next - Chat SDK Update

## Overview

Updated launchpad-next to use the new unified chat architecture from the JS SDK (`@raisindb/client`). The AI chat functionality now uses the new `ChatClient` and `ConversationClient` APIs in direct mode instead of the old flow-based approach.

## Key Changes

### 1. Node Types

**Before:**
- `raisin:AIConversation` for conversations
- `raisin:AIMessage` for messages
- Stored in `{user_home}/ai-chats/`

**After:**
- `raisin:Conversation` for all conversations
- `raisin:Message` for all messages
- Stored in `{user_home}/conversations/`
- Uses `conversation_type: 'ai_chat'` to distinguish AI chats from direct messages

### 2. SDK APIs Used

**Before:**
- Manual conversation creation with SQL INSERT
- Flow-based `ChatStore` with flow instance IDs
- `FlowClient` for managing flow instances

**After:**
- `ConversationClient` for listing/creating conversations
- `ChatClient` (via `ChatStore`) in direct mode for messaging
- No flow instances needed - conversations are node-based

### 3. Store Changes (`lib/stores/ai-chat.ts`)

#### Conversation Creation
```typescript
// OLD: Manual SQL INSERT
await db.executeSql(`INSERT INTO ...`);

// NEW: Use ConversationClient
const client = getClient();
const db = client.database('launchpad-next');
const conversationClient = db.conversations;
const conversation = await conversationClient.startAIChat({
  agent: '/agents/sample-assistant',
});
```

#### Listing Conversations
```typescript
// OLD: Direct SQL query for raisin:AIConversation
const rows = await query(`
  SELECT ... FROM 'raisin:access_control'
  WHERE node_type = 'raisin:AIConversation'
`);

// NEW: Use ConversationClient
const conversationList = await conversationClient.listConversations({
  type: 'ai_chat',
});
```

#### Sending Messages
```typescript
// OLD: Flow-based with instance IDs
const store = new ChatStore({ flow: '/flows/chat' });
const instanceId = store.getConversationId();
saveInstanceId(convId, instanceId);

// NEW: Direct mode with conversation paths
const store = new ChatStore({
  agent: '/agents/sample-assistant',
  database: sharedDb,
});
// Uses conversationPath instead of instanceId
```

### 4. Storage Paths

**Before:**
```
/raisin:access_control/users/{userId}/ai-chats/{chat-id}
  ├── {message-1} (raisin:AIMessage)
  └── {message-2} (raisin:AIMessage)
```

**After:**
```
/raisin:access_control/users/{userId}/conversations/{conversation-id}
  ├── {message-1} (raisin:Message, role: "user")
  ├── {message-2} (raisin:Message, role: "assistant")
  └── {message-3} (raisin:Message, role: "user")
```

## Testing the Changes

### 1. Create a New AI Chat
```typescript
// The store will:
1. Use ConversationClient.startAIChat() to create conversation
2. Create a ChatStore with agent path
3. Return the conversation path (not flow instance ID)
```

### 2. Send a Message
```typescript
// The ChatStore will:
1. Create a raisin:Message node with role: "user"
2. Subscribe to SSE events from the conversation
3. Stream AI response as text_chunk events
4. Create assistant message when complete
```

### 3. Load Message History
```typescript
// The store will:
1. Try ChatStore.loadMessages() first (uses ChatClient.getMessages())
2. Fallback to SQL query for raisin:Message nodes
```

## Agent Configuration

The default agent is configured in `ai-chat.ts`:

```typescript
export const DEFAULT_AGENT = {
  path: '/agents/sample-assistant',
  workspace: 'functions',
  name: 'Assistant'
};
```

This matches the agent in the `ai-tools` package at `/functions/agents/sample-assistant`.

## Compatibility Notes

### What Still Works
- All Svelte components remain unchanged
- Store interface is backward compatible
- Real-time subscriptions for new conversations
- Tool call tracking (create-plan, add-task, etc.)
- Streaming text with optimistic updates

### What Changed
- Conversation IDs are now paths (e.g., `/raisin:access_control/users/abc/conversations/chat-123`)
- localStorage key now stores paths instead of custom IDs
- No more flow instance ID tracking
- Conversations are always in direct mode (no flow fallback)

## Migration Path

If you have existing `raisin:AIConversation` nodes:

1. They will not appear in the new conversation list
2. Users should start fresh conversations with the new system
3. Old conversations can be migrated by:
   - Creating a new `raisin:Conversation` node
   - Copying messages as `raisin:Message` nodes with `role` property
   - Setting `conversation_type: 'ai_chat'`

## Next Steps

1. **Test the chat functionality** by creating a new conversation and sending messages
2. **Verify agent integration** by checking that `/agents/sample-assistant` responds correctly
3. **Check real-time updates** by having the agent initiate a conversation via triggers
4. **Review error handling** for network failures and SSE disconnections

## SDK Documentation

For more details on the new APIs, see:

- `packages/raisin-client-js/src/chat-client.ts` - ChatClient for messaging
- `packages/raisin-client-js/src/conversation-client.ts` - ConversationClient for management
- `packages/raisin-client-js/src/integrations/svelte-chat.ts` - ChatStore adapter
- `packages/raisin-client-js/IMPLEMENTATION_SUMMARY.md` - Full SDK documentation
