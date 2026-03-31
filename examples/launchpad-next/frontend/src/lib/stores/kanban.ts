/**
 * Kanban Store - Local kanban board state with flow-triggered AI chat.
 *
 * Manages columns and cards in-memory (no database persistence needed
 * for this demo). When a card is moved to the "Done" column, a flow
 * is triggered that starts an AI chat session asking the user about
 * writing a completion summary.
 */
import { browser } from '$app/environment';
import {
  ConversationStore,
  type ChatEvent,
} from '@raisindb/client';
import { getDatabase } from '$lib/raisin';

const STORAGE_KEY = 'launchpad-next:kanban:boards';

// ============================================================================
// Types
// ============================================================================

export interface KanbanCard {
  id: string;
  title: string;
  description?: string;
  summary?: string;
}

export interface KanbanColumn {
  id: string;
  title: string;
  cards: KanbanCard[];
}

export interface KanbanBoard {
  columns: KanbanColumn[];
}

export interface FlowChat {
  cardId: string;
  cardTitle: string;
  store: ConversationStore;
  messages: Array<{ role: string; content: string }>;
  isStreaming: boolean;
  isWaiting: boolean;
  streamingText: string;
  error: string | null;
}

type Subscriber = () => void;

// ============================================================================
// Default board data
// ============================================================================

function createDefaultBoard(): KanbanBoard {
  return {
    columns: [
      {
        id: 'todo',
        title: 'To Do',
        cards: [
          { id: 'card-1', title: 'Design landing page', description: 'Create wireframes and mockups' },
          { id: 'card-2', title: 'Set up CI/CD pipeline', description: 'Configure GitHub Actions' },
          { id: 'card-3', title: 'Write API documentation', description: 'Document all REST endpoints' },
        ],
      },
      {
        id: 'in-progress',
        title: 'In Progress',
        cards: [
          { id: 'card-4', title: 'Implement auth flow', description: 'Login, register, password reset' },
        ],
      },
      {
        id: 'done',
        title: 'Done',
        cards: [],
      },
    ],
  };
}

// ============================================================================
// Store
// ============================================================================

function createKanbanStore() {
  let board: KanbanBoard = createDefaultBoard();
  let activeFlowChat: FlowChat | null = null;
  let subscribers = new Set<Subscriber>();

  // Load persisted board from localStorage
  if (browser) {
    try {
      const stored = localStorage.getItem(STORAGE_KEY);
      if (stored) {
        board = JSON.parse(stored);
      }
    } catch { /* use default */ }
  }

  function notify() {
    for (const sub of subscribers) {
      try { sub(); } catch { /* ignore */ }
    }
  }

  function persist() {
    if (!browser) return;
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(board));
    } catch { /* ignore */ }
  }

  /**
   * Start a flow-triggered chat when a card moves to Done.
   * Uses ConversationStore with createOptions so the conversation is
   * created automatically on the first sendMessage() call.
   */
  async function startFlowChat(card: KanbanCard) {
    // Clean up previous chat
    if (activeFlowChat?.store) {
      activeFlowChat.store.destroy();
    }

    const db = await getDatabase();

    const store = new ConversationStore({
      database: db,
      createOptions: {
        participant: '/agents/sample-assistant',
      },
      onEvent: (_event: ChatEvent) => {
        // The ConversationStore subscribe callback handles all state updates.
        // This hook is available for additional event processing if needed.
      },
    });

    activeFlowChat = {
      cardId: card.id,
      cardTitle: card.title,
      store,
      messages: [],
      isStreaming: false,
      isWaiting: false,
      streamingText: '',
      error: null,
    };

    // Subscribe to store state changes
    store.subscribe((snapshot) => {
      if (!activeFlowChat) return;
      activeFlowChat.messages = snapshot.messages.map(m => ({
        role: m.role,
        content: m.content,
      }));
      activeFlowChat.isStreaming = snapshot.isStreaming;
      activeFlowChat.isWaiting = snapshot.isWaiting;
      activeFlowChat.streamingText = snapshot.streamingText;
      activeFlowChat.error = snapshot.error;
      notify();
    });

    notify();

    // Send initial message to kick off the conversation
    const prompt = `The task "${card.title}" has been marked as done.${card.description ? ` Description: ${card.description}` : ''} Would you like me to write a completion summary for this task?`;
    await store.sendMessage(prompt);
  }

  return {
    subscribe(callback: Subscriber): () => void {
      subscribers.add(callback);
      return () => { subscribers.delete(callback); };
    },

    getBoard(): KanbanBoard {
      return board;
    },

    getActiveFlowChat(): FlowChat | null {
      return activeFlowChat;
    },

    addCard(columnId: string, title: string, description?: string) {
      const column = board.columns.find(c => c.id === columnId);
      if (!column) return;
      column.cards.push({
        id: `card-${Date.now()}`,
        title,
        description,
      });
      board = { ...board };
      persist();
      notify();
    },

    deleteCard(columnId: string, cardId: string) {
      const column = board.columns.find(c => c.id === columnId);
      if (!column) return;
      column.cards = column.cards.filter(c => c.id !== cardId);
      board = { ...board };
      persist();
      notify();
    },

    moveCard(cardId: string, fromColumnId: string, toColumnId: string) {
      const fromColumn = board.columns.find(c => c.id === fromColumnId);
      const toColumn = board.columns.find(c => c.id === toColumnId);
      if (!fromColumn || !toColumn) return;

      const cardIndex = fromColumn.cards.findIndex(c => c.id === cardId);
      if (cardIndex === -1) return;

      const [card] = fromColumn.cards.splice(cardIndex, 1);
      toColumn.cards.push(card);
      board = { ...board };
      persist();
      notify();

      // Trigger AI chat when card moves to Done
      if (toColumnId === 'done') {
        startFlowChat(card);
      }
    },

    async sendChatMessage(content: string) {
      if (!activeFlowChat?.store) return;
      await activeFlowChat.store.sendMessage(content);
    },

    dismissFlowChat() {
      if (activeFlowChat?.store) {
        activeFlowChat.store.destroy();
      }
      activeFlowChat = null;
      notify();
    },

    updateCardSummary(cardId: string, summary: string) {
      for (const column of board.columns) {
        const card = column.cards.find(c => c.id === cardId);
        if (card) {
          card.summary = summary;
          board = { ...board };
          persist();
          notify();
          return;
        }
      }
    },

    resetBoard() {
      board = createDefaultBoard();
      if (activeFlowChat?.store) {
        activeFlowChat.store.destroy();
      }
      activeFlowChat = null;
      persist();
      notify();
    },
  };
}

export const kanbanStore = createKanbanStore();
