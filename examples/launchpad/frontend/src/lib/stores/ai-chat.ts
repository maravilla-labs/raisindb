/**
 * AI Chat Store - Manages AI Agent Conversations
 *
 * This store provides:
 * - AI conversation creation and management
 * - Real-time updates via WebSocket subscriptions
 * - Message sending and response handling
 * - Support for tool calls, thoughts, and multi-turn conversations
 *
 * Architecture:
 * - Conversations are stored in user's home workspace: /users/{userId}/ai-chats/
 * - Messages are AIMessage nodes with role: user|assistant|system
 * - Responses are triggered automatically by the on-user-message trigger in ai-tools package
 */
import { writable, derived, get } from 'svelte/store';
import { browser } from '$app/environment';
import { getDatabase, onReconnected, query } from '$lib/raisin';
import { user } from './auth';

const ACCESS_CONTROL = 'raisin:access_control';
const LAST_CONVERSATION_KEY = 'launchpad:ai-chat:lastConversationId';

// Default agent configuration - uses sample-assistant from ai-tools package
export const DEFAULT_AGENT = {
  path: '/agents/sample-assistant',
  workspace: 'functions',
  name: 'Assistant'
};

// ============================================================================
// Types
// ============================================================================

export interface AIConversation {
  id: string;
  path: string;
  agentRef: AgentReference;
  title: string;
  status: string;
  createdAt: string;
  updatedAt: string;
  messageCount: number;
}

export interface AgentReference {
  'raisin:ref': string;
  'raisin:workspace': string;
  'raisin:path': string;
}

export interface AIMessage {
  id: string;
  path: string;
  name: string;
  role: 'user' | 'assistant' | 'system';
  content: string;
  finishReason?: string;
  timestamp: string;
  children?: AIMessageChild[];
  _optimistic?: boolean;
}

export interface AIMessageChild {
  id: string;
  type: 'thought' | 'tool_call' | 'tool_result';
  content: string;
  toolName?: string;
  toolInput?: unknown;
  status?: string;
  expanded?: boolean;
}

export interface AIAgent {
  id: string;
  path: string;
  name: string;
  systemPrompt?: string;
  model?: string;
  provider?: string;
}

export interface AIContext {
  type: 'board' | 'page' | 'general';
  path: string;
  boardSlug?: string;
}

interface AIChatState {
  // Conversation data
  conversations: Map<string, AIConversation>;
  messages: Map<string, AIMessage[]>;

  // Available agents
  agents: AIAgent[];

  // Active conversation
  activeConversationId: string | null;

  // UI state
  isOpen: boolean;
  isMinimized: boolean;
  isWaitingForResponse: boolean;

  // Loading states
  loading: boolean;
  loadingAgents: boolean;
  error: string | null;

  // Subscription state
  subscribed: boolean;
  initialized: boolean;

  // Voice activation context
  context: AIContext | null;
}

// ============================================================================
// Store Implementation
// ============================================================================

const initialState: AIChatState = {
  conversations: new Map(),
  messages: new Map(),
  agents: [],
  activeConversationId: null,
  isOpen: false,
  isMinimized: false,
  isWaitingForResponse: false,
  loading: false,
  loadingAgents: false,
  error: null,
  subscribed: false,
  initialized: false,
  context: null,
};

let unsubscribeEvents: (() => void) | null = null;
let unsubscribeReconnected: (() => void) | null = null;

// localStorage helpers
function saveLastConversationId(convId: string | null) {
  if (!browser) return;
  if (convId) {
    localStorage.setItem(LAST_CONVERSATION_KEY, convId);
  } else {
    localStorage.removeItem(LAST_CONVERSATION_KEY);
  }
}

function loadLastConversationId(): string | null {
  if (!browser) return null;
  return localStorage.getItem(LAST_CONVERSATION_KEY);
}

function createAIChatStore() {
  const { subscribe, set, update } = writable<AIChatState>({ ...initialState });

  /**
   * Parse event path to extract conversation and message info
   */
  function parseEventPath(path: string): {
    convId: string | null;
    msgName: string | null;
    isConversation: boolean;
    isMessage: boolean;
    isChild: boolean;
  } {
    // Path patterns:
    // Conversation: /users/{userId}/ai-chats/{convId}
    // Message: /users/{userId}/ai-chats/{convId}/msg-xxx
    // Child: /users/{userId}/ai-chats/{convId}/msg-xxx/thought-xxx or /msg-xxx/tool-call-xxx/...
    const parts = path.split('/');
    const aiChatsIndex = parts.indexOf('ai-chats');

    if (aiChatsIndex === -1 || parts.length <= aiChatsIndex + 1) {
      return { convId: null, msgName: null, isConversation: false, isMessage: false, isChild: false };
    }

    const convId = parts[aiChatsIndex + 1];
    const isConversation = parts.length === aiChatsIndex + 2;
    const isMessage = parts.length === aiChatsIndex + 3;
    const isChild = parts.length > aiChatsIndex + 3;
    const msgName = parts.length > aiChatsIndex + 2 ? parts[aiChatsIndex + 2] : null;

    return { convId, msgName, isConversation, isMessage, isChild };
  }

  /**
   * Handle incoming WebSocket events
   */
  function handleEvent(event: any) {
    if (!event) return;

    const payload = event.payload;
    const kind = payload?.kind as 'Created' | 'Updated' | 'Deleted';
    const path = payload?.path as string;
    const node = payload?.node;

    if (!path) return;

    const { convId, msgName, isMessage, isConversation, isChild } = parseEventPath(path);

    if (kind === 'Created' && node) {
      if (isMessage && convId && node.node_type === 'raisin:AIMessage') {
        handleNewMessage(convId, node);
      } else if (isConversation && node.node_type === 'raisin:AIConversation') {
        handleNewConversation(node);
      } else if (isChild && convId && msgName) {
        // Handle child nodes (thoughts, tool calls, tool results)
        handleNewChild(convId, msgName, node);
      }
    } else if (kind === 'Updated' && node) {
      if (isMessage && convId && node.node_type === 'raisin:AIMessage') {
        handleMessageUpdate(convId, node);
      } else if (isConversation && node.node_type === 'raisin:AIConversation') {
        handleConversationUpdate(node);
      } else if (isChild && convId && msgName) {
        // Handle child updates (e.g., tool call status changes)
        handleChildUpdate(convId, msgName, node);
      }
    }
  }

  /**
   * Handle new AI message from assistant
   */
  function handleNewMessage(convId: string, node: any) {
    const role = node.properties?.role as 'user' | 'assistant' | 'system';
    const content = node.properties?.content || '';
    const finishReason = node.properties?.finish_reason;

    update(state => {
      const existingMessages = state.messages.get(convId) || [];

      // Check for duplicates
      if (existingMessages.some(m => m.id === node.id || m.path === node.path)) {
        return state;
      }

      // Remove optimistic message if this is the server response
      const clientId = node.properties?.client_id;
      const filteredMessages = existingMessages.filter(m => {
        if (m._optimistic && clientId && m.id === clientId) return false;
        return true;
      });

      const message: AIMessage = {
        id: node.id,
        path: node.path,
        name: node.name,
        role,
        content,
        finishReason,
        timestamp: node.created_at || new Date().toISOString(),
        children: [],
      };

      const newMessages = new Map(state.messages);
      newMessages.set(convId, [...filteredMessages, message]);

      // Check if we should stop waiting (both 'stop' and 'error' are terminal states)
      const isComplete = role === 'assistant' &&
                         (finishReason === 'stop' || finishReason === 'error');

      return {
        ...state,
        messages: newMessages,
        isWaitingForResponse: isComplete ? false : state.isWaitingForResponse,
      };
    });
  }

  /**
   * Handle message update (e.g., when response is complete)
   */
  function handleMessageUpdate(convId: string, node: any) {
    update(state => {
      const messages = state.messages.get(convId);
      if (!messages) return state;

      const updatedMessages = messages.map(m => {
        if (m.id === node.id) {
          return {
            ...m,
            content: node.properties?.content || m.content,
            finishReason: node.properties?.finish_reason || m.finishReason,
          };
        }
        return m;
      });

      const newMessages = new Map(state.messages);
      newMessages.set(convId, updatedMessages);

      // Check if response is complete (both 'stop' and 'error' are terminal states)
      const lastMsg = updatedMessages[updatedMessages.length - 1];
      const isComplete = lastMsg?.role === 'assistant' &&
                         (lastMsg?.finishReason === 'stop' || lastMsg?.finishReason === 'error');

      return {
        ...state,
        messages: newMessages,
        isWaitingForResponse: isComplete ? false : state.isWaitingForResponse,
      };
    });
  }

  /**
   * Handle new child node (thought, tool call, or tool result)
   */
  function handleNewChild(convId: string, msgName: string, node: any) {
    const nodeType = node.node_type as string;

    // Parse child info based on node type
    let child: AIMessageChild | null = null;

    if (nodeType === 'raisin:AIThought') {
      child = {
        id: node.id,
        type: 'thought',
        content: node.properties?.content || '',
      };
    } else if (nodeType === 'raisin:AIToolCall') {
      const functionRef = node.properties?.function_ref;
      const toolName = functionRef?.['raisin:path']?.split('/').pop() || 'unknown';

      child = {
        id: node.id,
        type: 'tool_call',
        content: `Status: ${node.properties?.status || 'pending'}`,
        toolName,
        toolInput: node.properties?.arguments,
        status: node.properties?.status || 'pending',
      };
    } else if (nodeType === 'raisin:AIToolResult' || nodeType === 'raisin:AIToolSingleCallResult') {
      // Extract tool name from path if possible
      const pathParts = (node.path as string).split('/');
      const toolCallIndex = pathParts.findIndex(p => p.startsWith('tool-call-'));
      const toolName = toolCallIndex > 0 ? pathParts[toolCallIndex].replace('tool-call-', '') : undefined;

      child = {
        id: node.id,
        type: 'tool_result',
        content: JSON.stringify(node.properties?.result ?? node.properties?.error ?? '', null, 2),
        toolName,
      };
    }

    if (!child) return;

    update(state => {
      const messages = state.messages.get(convId);
      if (!messages) return state;

      const updatedMessages = messages.map(m => {
        if (m.name === msgName) {
          // Check for duplicate
          if (m.children?.some(c => c.id === child!.id)) {
            return m;
          }
          return {
            ...m,
            children: [...(m.children || []), child!],
          };
        }
        return m;
      });

      const newMessages = new Map(state.messages);
      newMessages.set(convId, updatedMessages);

      return { ...state, messages: newMessages };
    });
  }

  /**
   * Handle child node update (e.g., tool call status change)
   */
  function handleChildUpdate(convId: string, msgName: string, node: any) {
    const nodeType = node.node_type as string;

    if (nodeType !== 'raisin:AIToolCall') return;

    update(state => {
      const messages = state.messages.get(convId);
      if (!messages) return state;

      const updatedMessages = messages.map(m => {
        if (m.name === msgName && m.children) {
          return {
            ...m,
            children: m.children.map(c => {
              if (c.id === node.id && c.type === 'tool_call') {
                return {
                  ...c,
                  status: node.properties?.status || c.status,
                  content: `Status: ${node.properties?.status || c.status}`,
                };
              }
              return c;
            }),
          };
        }
        return m;
      });

      const newMessages = new Map(state.messages);
      newMessages.set(convId, updatedMessages);

      return { ...state, messages: newMessages };
    });
  }

  /**
   * Handle new conversation created
   */
  function handleNewConversation(node: any) {
    const conv: AIConversation = {
      id: node.name,
      path: node.path,
      agentRef: node.properties?.agent_ref,
      title: node.properties?.title || 'New Chat',
      status: node.properties?.status || 'active',
      createdAt: node.created_at || new Date().toISOString(),
      updatedAt: node.updated_at || node.created_at || new Date().toISOString(),
      messageCount: 0,
    };

    update(state => {
      const conversations = new Map(state.conversations);
      conversations.set(conv.id, conv);
      return { ...state, conversations };
    });
  }

  /**
   * Handle conversation update
   */
  function handleConversationUpdate(node: any) {
    update(state => {
      const conv = state.conversations.get(node.name);
      if (!conv) return state;

      const conversations = new Map(state.conversations);
      conversations.set(node.name, {
        ...conv,
        status: node.properties?.status ?? conv.status,
        messageCount: node.properties?.message_count ?? conv.messageCount,
      });

      return { ...state, conversations };
    });
  }

  /**
   * Setup WebSocket subscription for AI chat events
   */
  async function setupSubscription(aiChatsPath: string) {
    try {
      const db = await getDatabase();
      const ws = db.workspace(ACCESS_CONTROL);

      const subscription = await ws.events().subscribeToPath(
        `${aiChatsPath}/**`,
        (event) => {
          try {
            handleEvent(event);
          } catch (err) {
            console.error('[ai-chat] Error handling event:', err);
          }
        },
        { includeNode: true }
      );

      unsubscribeEvents = () => subscription.unsubscribe();
      update(state => ({ ...state, subscribed: true }));
    } catch (err) {
      console.error('[ai-chat] Subscription failed:', err);
    }
  }

  /**
   * Setup reconnection listener
   */
  function setupReconnectedListener() {
    unsubscribeReconnected = onReconnected(() => {
      console.log('[ai-chat] Reconnected, syncing data...');
      const state = get({ subscribe });
      if (state.activeConversationId) {
        loadConversationMessages(state.activeConversationId);
      }
    });
  }

  /**
   * Load messages for a conversation including children (thoughts, tool calls)
   */
  async function loadConversationMessages(convId: string) {
    const state = get({ subscribe });
    const conv = state.conversations.get(convId);
    if (!conv) return [];

    try {
      // Load all messages
      const msgRows = await query<any>(`
        SELECT id, path, name, node_type, properties, created_at
        FROM '${ACCESS_CONTROL}'
        WHERE CHILD_OF('${conv.path}')
          AND node_type = 'raisin:AIMessage'
        ORDER BY created_at ASC
      `);

      // Load all descendants in one query for efficiency
      const allDescendants = await query<any>(`
        SELECT id, path, name, node_type, properties, created_at
        FROM '${ACCESS_CONTROL}'
        WHERE DESCENDANT_OF('${conv.path}')
          AND node_type IN ('raisin:AIThought', 'raisin:AIToolCall', 'raisin:AIToolResult', 'raisin:AIToolSingleCallResult')
        ORDER BY created_at ASC
      `);

      // Group descendants by their parent message
      const childrenByMsgPath = new Map<string, any[]>();
      const toolResultsByToolCallPath = new Map<string, any[]>();

      for (const desc of allDescendants) {
        const pathParts = desc.path.split('/');
        // Find the message part of the path (msg-xxx)
        const msgIndex = pathParts.findIndex((p: string) => p.startsWith('msg-'));
        if (msgIndex === -1) continue;

        const msgPath = pathParts.slice(0, msgIndex + 1).join('/');

        // Check if this is a tool result (child of tool call)
        if (desc.node_type === 'raisin:AIToolResult' || desc.node_type === 'raisin:AIToolSingleCallResult') {
          const toolCallPath = pathParts.slice(0, -1).join('/');
          if (!toolResultsByToolCallPath.has(toolCallPath)) {
            toolResultsByToolCallPath.set(toolCallPath, []);
          }
          toolResultsByToolCallPath.get(toolCallPath)!.push(desc);
        } else {
          // Direct child of message (thought or tool call)
          if (!childrenByMsgPath.has(msgPath)) {
            childrenByMsgPath.set(msgPath, []);
          }
          childrenByMsgPath.get(msgPath)!.push(desc);
        }
      }

      // Build messages with children
      const messages: AIMessage[] = [];

      for (const row of msgRows) {
        const children: AIMessageChild[] = [];
        const msgChildren = childrenByMsgPath.get(row.path) || [];

        for (const child of msgChildren) {
          if (child.node_type === 'raisin:AIThought') {
            children.push({
              id: child.id,
              type: 'thought',
              content: child.properties?.content || '',
            });
          } else if (child.node_type === 'raisin:AIToolCall') {
            const functionRef = child.properties?.function_ref;
            const toolName = functionRef?.['raisin:path']?.split('/').pop() || 'unknown';

            children.push({
              id: child.id,
              type: 'tool_call',
              content: `Status: ${child.properties?.status || 'pending'}`,
              toolName,
              toolInput: child.properties?.arguments,
              status: child.properties?.status || 'pending',
            });

            // Add tool results for this tool call
            const toolResults = toolResultsByToolCallPath.get(child.path) || [];
            for (const result of toolResults) {
              children.push({
                id: result.id,
                type: 'tool_result',
                content: JSON.stringify(result.properties?.result ?? result.properties?.error ?? '', null, 2),
                toolName,
              });
            }
          }
        }

        messages.push({
          id: row.id,
          path: row.path,
          name: row.name,
          role: row.properties?.role || 'user',
          content: row.properties?.content || '',
          finishReason: row.properties?.finish_reason,
          timestamp: row.created_at,
          children,
        });
      }

      update(s => {
        const newMessages = new Map(s.messages);
        newMessages.set(convId, messages);

        // Check if waiting for response (both 'stop' and 'error' are terminal states)
        const lastMsg = messages[messages.length - 1];
        const isWaiting = lastMsg?.role === 'user' ||
                          (lastMsg?.role === 'assistant' &&
                           lastMsg?.finishReason !== 'stop' &&
                           lastMsg?.finishReason !== 'error');

        return {
          ...s,
          messages: newMessages,
          isWaitingForResponse: isWaiting,
        };
      });

      return messages;
    } catch (err) {
      console.error('[ai-chat] Failed to load messages:', err);
      return [];
    }
  }

  return {
    subscribe,

    /**
     * Initialize the store
     */
    async init() {
      const state = get({ subscribe });
      if (state.initialized) return;
      if (!browser) return;

      const currentUser = get(user);
      if (!currentUser?.home) return;

      const homePath = currentUser.home.replace(`/${ACCESS_CONTROL}`, '');
      const aiChatsPath = `${homePath}/ai-chats`;

      update(s => ({ ...s, loading: true, error: null }));

      try {
        // Load available agents from functions workspace
        await this.loadAgents();

        // Load existing conversations
        const convRows = await query<any>(`
          SELECT id, path, name, node_type, properties, created_at, updated_at
          FROM '${ACCESS_CONTROL}'
          WHERE CHILD_OF('${aiChatsPath}')
            AND node_type = 'raisin:AIConversation'
          ORDER BY updated_at DESC
        `);

        const conversations = new Map<string, AIConversation>();
        for (const row of convRows) {
          conversations.set(row.name, {
            id: row.name,
            path: row.path,
            agentRef: row.properties?.agent_ref,
            title: row.properties?.title || 'Chat',
            status: row.properties?.status || 'active',
            createdAt: row.created_at,
            updatedAt: row.updated_at || row.created_at,
            messageCount: row.properties?.message_count || 0,
          });
        }

        // Restore last active conversation from localStorage
        const lastConvId = loadLastConversationId();
        let activeConversationId: string | null = null;

        if (lastConvId && conversations.has(lastConvId)) {
          activeConversationId = lastConvId;
        } else if (conversations.size > 0) {
          // Select the most recently updated conversation
          const sorted = Array.from(conversations.values())
            .sort((a, b) => new Date(b.updatedAt).getTime() - new Date(a.updatedAt).getTime());
          activeConversationId = sorted[0]?.id || null;
        }

        update(s => ({
          ...s,
          conversations,
          activeConversationId,
          loading: false,
          initialized: true,
        }));

        // Load messages for active conversation
        if (activeConversationId) {
          await loadConversationMessages(activeConversationId);
        }

        // Setup subscriptions
        await setupSubscription(aiChatsPath);
        setupReconnectedListener();
      } catch (err) {
        console.error('[ai-chat] Init failed:', err);
        update(s => ({
          ...s,
          loading: false,
          error: 'Failed to initialize AI chat',
        }));
      }
    },

    /**
     * Load available agents from functions workspace
     */
    async loadAgents() {
      update(s => ({ ...s, loadingAgents: true }));

      try {
        const rows = await query<any>(`
          SELECT id, path, name, properties
          FROM 'functions'
          WHERE node_type = 'raisin:AIAgent'
        `);

        const agents: AIAgent[] = rows.map(row => ({
          id: row.id,
          path: row.path,
          name: row.name,
          systemPrompt: row.properties?.system_prompt,
          model: row.properties?.model,
          provider: row.properties?.provider,
        }));

        update(s => ({ ...s, agents, loadingAgents: false }));
        return agents;
      } catch (err) {
        console.error('[ai-chat] Failed to load agents:', err);
        update(s => ({ ...s, loadingAgents: false }));
        return [];
      }
    },

    /**
     * Create a new conversation with an agent
     */
    async createConversation(agentPath: string = DEFAULT_AGENT.path): Promise<string | null> {
      const currentUser = get(user);
      if (!currentUser?.home) return null;

      const homePath = currentUser.home.replace(`/${ACCESS_CONTROL}`, '');
      const aiChatsPath = `${homePath}/ai-chats`;

      // Find the agent
      const state = get({ subscribe });
      const agent = state.agents.find(a => a.path === `/functions${agentPath}` || a.path === agentPath);
      const agentName = agent?.name || 'assistant';

      const convName = `chat-${agentName}-${Date.now()}`;

      try {
        const db = await getDatabase();

        // First check if ai-chats folder exists, create if not
        const folderExists = await query<any>(`
          SELECT id FROM '${ACCESS_CONTROL}'
          WHERE path = '${aiChatsPath}'
          LIMIT 1
        `);

        if (folderExists.length === 0) {
          await db.executeSql(`
            INSERT INTO '${ACCESS_CONTROL}' (path, node_type, properties)
            VALUES ($1, 'raisin:Folder', $2::jsonb)
          `, [aiChatsPath, JSON.stringify({ name: 'ai-chats' })]);
        }

        // Create the conversation
        const convPath = `${aiChatsPath}/${convName}`;
        await db.executeSql(`
          INSERT INTO '${ACCESS_CONTROL}' (path, node_type, properties)
          VALUES ($1, 'raisin:AIConversation', $2::jsonb)
        `, [convPath, JSON.stringify({
          agent_ref: {
            'raisin:ref': agent?.id || '',
            'raisin:workspace': 'functions',
            'raisin:path': agentPath,
          },
          status: 'active',
          title: `Chat with ${agentName}`,
        })]);

        // Add to local state
        const now = new Date().toISOString();
        const conv: AIConversation = {
          id: convName,
          path: convPath,
          agentRef: {
            'raisin:ref': agent?.id || '',
            'raisin:workspace': 'functions',
            'raisin:path': agentPath,
          },
          title: `Chat with ${agentName}`,
          status: 'active',
          createdAt: now,
          updatedAt: now,
          messageCount: 0,
        };

        update(s => {
          const conversations = new Map(s.conversations);
          conversations.set(convName, conv);
          return {
            ...s,
            conversations,
            activeConversationId: convName,
            messages: new Map(s.messages).set(convName, []),
          };
        });

        saveLastConversationId(convName);
        return convName;
      } catch (err) {
        console.error('[ai-chat] Failed to create conversation:', err);
        return null;
      }
    },

    /**
     * Send a message to the active conversation
     */
    async sendMessage(content: string): Promise<boolean> {
      const state = get({ subscribe });
      if (!state.activeConversationId) return false;

      const conv = state.conversations.get(state.activeConversationId);
      if (!conv) return false;

      const msgName = `msg-${Date.now()}`;
      const msgPath = `${conv.path}/${msgName}`;
      const clientId = crypto.randomUUID();
      const now = new Date().toISOString();

      // Optimistic update
      const optimisticMessage: AIMessage = {
        id: clientId,
        path: msgPath,
        name: msgName,
        role: 'user',
        content,
        timestamp: now,
        _optimistic: true,
      };

      update(s => {
        const messages = s.messages.get(state.activeConversationId!) || [];
        const newMessages = new Map(s.messages);
        newMessages.set(state.activeConversationId!, [...messages, optimisticMessage]);
        return {
          ...s,
          messages: newMessages,
          isWaitingForResponse: true,
        };
      });

      // Send to server
      try {
        const db = await getDatabase();
        await db.executeSql(`
          INSERT INTO '${ACCESS_CONTROL}' (path, node_type, properties)
          VALUES ($1, 'raisin:AIMessage', $2::jsonb)
        `, [msgPath, JSON.stringify({
          role: 'user',
          content,
          client_id: clientId,
        })]);

        return true;
      } catch (err) {
        console.error('[ai-chat] Failed to send message:', err);
        // Remove optimistic message on error
        update(s => {
          const messages = s.messages.get(state.activeConversationId!) || [];
          const filtered = messages.filter(m => m.id !== clientId);
          const newMessages = new Map(s.messages);
          newMessages.set(state.activeConversationId!, filtered);
          return {
            ...s,
            messages: newMessages,
            isWaitingForResponse: false,
            error: 'Failed to send message',
          };
        });
        return false;
      }
    },

    /**
     * Set the active conversation
     */
    async setActiveConversation(convId: string | null) {
      update(s => ({ ...s, activeConversationId: convId }));
      saveLastConversationId(convId);

      if (convId) {
        await loadConversationMessages(convId);
      }
    },

    /**
     * Delete a conversation
     */
    async deleteConversation(convId: string): Promise<boolean> {
      const state = get({ subscribe });
      const conv = state.conversations.get(convId);
      if (!conv) return false;

      try {
        const db = await getDatabase();

        // Delete all children (messages, etc.) first, then the conversation
        await db.executeSql(`
          DELETE FROM '${ACCESS_CONTROL}'
          WHERE path = $1 OR path LIKE $2
        `, [conv.path, `${conv.path}/%`]);

        // Update local state
        update(s => {
          const conversations = new Map(s.conversations);
          conversations.delete(convId);

          const messages = new Map(s.messages);
          messages.delete(convId);

          // If we deleted the active conversation, clear it
          let newActiveId = s.activeConversationId;
          if (s.activeConversationId === convId) {
            // Try to select the most recently updated remaining conversation
            const remaining = Array.from(conversations.values())
              .sort((a, b) => new Date(b.updatedAt).getTime() - new Date(a.updatedAt).getTime());
            newActiveId = remaining.length > 0 ? remaining[0].id : null;
            saveLastConversationId(newActiveId);
          }

          return {
            ...s,
            conversations,
            messages,
            activeConversationId: newActiveId,
          };
        });

        // Load messages for new active conversation if one was selected
        const newState = get({ subscribe });
        if (newState.activeConversationId && newState.activeConversationId !== convId) {
          await loadConversationMessages(newState.activeConversationId);
        }

        return true;
      } catch (err) {
        console.error('[ai-chat] Failed to delete conversation:', err);
        return false;
      }
    },

    /**
     * Set context for the AI chat (e.g., current board)
     * This context will be included in messages to help the AI understand the user's context
     */
    setContext(context: AIContext | null) {
      update(s => ({ ...s, context }));
    },

    /**
     * Get current context
     */
    getContext(): AIContext | null {
      return get({ subscribe }).context;
    },

    /**
     * Open the chat widget
     */
    open() {
      update(s => ({ ...s, isOpen: true, isMinimized: false }));
    },

    /**
     * Close the chat widget
     */
    close() {
      update(s => ({ ...s, isOpen: false }));
    },

    /**
     * Toggle the chat widget
     */
    toggle() {
      update(s => ({ ...s, isOpen: !s.isOpen, isMinimized: false }));
    },

    /**
     * Minimize the chat widget
     */
    minimize() {
      update(s => ({ ...s, isMinimized: true }));
    },

    /**
     * Restore from minimized
     */
    restore() {
      update(s => ({ ...s, isMinimized: false }));
    },

    /**
     * Clear error
     */
    clearError() {
      update(s => ({ ...s, error: null }));
    },

    /**
     * Reset the store
     */
    reset() {
      if (unsubscribeEvents) {
        unsubscribeEvents();
        unsubscribeEvents = null;
      }
      if (unsubscribeReconnected) {
        unsubscribeReconnected();
        unsubscribeReconnected = null;
      }
      set({ ...initialState });
    },
  };
}

export const aiChatStore = createAIChatStore();

// ============================================================================
// Derived Stores
// ============================================================================

export const aiConversations = derived(aiChatStore, $s => $s.conversations);
export const aiMessages = derived(aiChatStore, $s => $s.messages);
export const aiAgents = derived(aiChatStore, $s => $s.agents);
export const activeAIConversationId = derived(aiChatStore, $s => $s.activeConversationId);
export const isAIChatOpen = derived(aiChatStore, $s => $s.isOpen);
export const isAIChatMinimized = derived(aiChatStore, $s => $s.isMinimized);
export const isWaitingForAIResponse = derived(aiChatStore, $s => $s.isWaitingForResponse);
export const aiChatLoading = derived(aiChatStore, $s => $s.loading);
export const aiChatError = derived(aiChatStore, $s => $s.error);

// Helper to get active conversation
export const activeAIConversation = derived(
  [aiChatStore],
  ([$s]) => $s.activeConversationId ? $s.conversations.get($s.activeConversationId) : null
);

// Helper to get active conversation messages
export const activeAIMessages = derived(
  [aiChatStore],
  ([$s]) => $s.activeConversationId ? ($s.messages.get($s.activeConversationId) || []) : []
);

// Current context (e.g., board path from voice activation)
export const aiChatContext = derived(aiChatStore, $s => $s.context);
