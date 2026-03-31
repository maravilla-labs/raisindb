import * as THREE from 'three';
import { getClient, getDatabase } from '$lib/raisin';
import type { PageNode } from '$lib/raisin';
import { Column3D, createColumn3D, type Column3DData } from './column-3d';
import type { Card3D } from './card-3d';
import type { HandInteraction, InteractableObject } from './hand-interaction';

export interface KanbanColumn {
  id: string;
  title: string;
  cards: Array<{
    uuid: string;
    element_type: string;
    title: string;
    description?: string;
  }>;
}

export interface BoardSyncConfig {
  columnGap?: number;
}

const DEFAULT_CONFIG: Required<BoardSyncConfig> = {
  columnGap: 0.35  // 35cm between columns
};

export class BoardSync {
  private board: PageNode;
  private boardGroup: THREE.Group;
  private columns: Map<string, Column3D> = new Map();
  private config: Required<BoardSyncConfig>;
  private handInteraction: HandInteraction | null = null;
  private unsubscribe: (() => void) | null = null;

  // Callbacks
  onCardMoved?: (cardId: string, fromColumnId: string, toColumnId: string, newIndex: number) => void;
  onBoardUpdated?: () => void;

  constructor(board: PageNode, boardGroup: THREE.Group, config: BoardSyncConfig = {}) {
    this.board = board;
    this.boardGroup = boardGroup;
    this.config = { ...DEFAULT_CONFIG, ...config };
  }

  async initialize() {
    // Create initial columns
    this.createColumnsFromBoard();

    // Subscribe to real-time updates
    await this.subscribeToUpdates();
  }

  setHandInteraction(handInteraction: HandInteraction) {
    this.handInteraction = handInteraction;

    // Register all cards as interactable
    for (const column of this.columns.values()) {
      for (const card of column.getAllCards()) {
        this.handInteraction.registerInteractable(
          card.mesh,
          card.data.uuid,
          { columnId: card.data.columnId, index: card.data.index }
        );
      }
    }

    // Handle hover for visual feedback
    this.handInteraction.onHover = (object) => {
      for (const column of this.columns.values()) {
        for (const card of column.getAllCards()) {
          card.setHovered(object?.id === card.data.uuid);
        }
      }
    };

    // Handle selection (pinch on card)
    this.handInteraction.onSelect = (object) => {
      const card = this.findCard(object.id);
      if (card) {
        card.setGrabbed(true);
        console.log('[BoardSync] Card selected:', card.data.title);
      }
    };

    // Handle deselection (release pinch)
    this.handInteraction.onDeselect = (object) => {
      const card = this.findCard(object.id);
      if (card) {
        card.setGrabbed(false);
        console.log('[BoardSync] Card deselected:', card.data.title);
      }
    };
  }

  private createColumnsFromBoard() {
    // Clear existing columns
    for (const column of this.columns.values()) {
      this.boardGroup.remove(column.group);
      column.dispose();
    }
    this.columns.clear();

    const boardColumns: KanbanColumn[] = (this.board.properties as any).columns ?? [];

    // Calculate total width for centering
    const columnWidth = 0.3;
    const totalWidth = boardColumns.length * columnWidth + (boardColumns.length - 1) * this.config.columnGap;
    const startX = -totalWidth / 2 + columnWidth / 2;

    boardColumns.forEach((columnData, index) => {
      const column3DData: Column3DData = {
        id: columnData.id,
        title: columnData.title,
        cards: columnData.cards.map(c => ({
          uuid: c.uuid,
          title: c.title,
          description: c.description
        }))
      };

      const column = createColumn3D(column3DData);

      // Position column
      const x = startX + index * (columnWidth + this.config.columnGap);
      column.group.position.set(x, 0, 0);

      this.columns.set(columnData.id, column);
      this.boardGroup.add(column.group);
    });
  }

  private async subscribeToUpdates() {
    try {
      const client = getClient();
      const db = client.database('launchpad-next');
      const workspace = db.workspace('launchpad');
      const events = workspace.events();

      const subscription = await events.subscribe(
        {
          workspace: 'launchpad',
          path: this.board.path,
          event_types: ['node:updated'],
        },
        async (event) => {
          // Re-fetch board data
          const result = await db.executeSql(
            'SELECT properties FROM launchpad WHERE path = $1 LIMIT 1',
            [this.board.path]
          );

          if (result.rows && result.rows.length > 0) {
            const newProperties = (result.rows[0] as any).properties;
            (this.board.properties as any).columns = newProperties.columns;
            this.createColumnsFromBoard();

            // Re-register interactables
            if (this.handInteraction) {
              this.handInteraction.clearInteractables();
              for (const column of this.columns.values()) {
                for (const card of column.getAllCards()) {
                  this.handInteraction.registerInteractable(
                    card.mesh,
                    card.data.uuid,
                    { columnId: card.data.columnId, index: card.data.index }
                  );
                }
              }
            }

            this.onBoardUpdated?.();
          }
        }
      );

      this.unsubscribe = () => subscription.unsubscribe();
    } catch (error) {
      console.error('[BoardSync] Failed to subscribe:', error);
    }
  }

  private findCard(cardId: string): Card3D | undefined {
    for (const column of this.columns.values()) {
      const card = column.getCard(cardId);
      if (card) return card;
    }
    return undefined;
  }

  private async saveCardMove(
    cardId: string,
    fromColumnId: string,
    toColumnId: string,
    toIndex: number
  ) {
    try {
      const db = await getDatabase();

      // Get current columns
      const columns: KanbanColumn[] = JSON.parse(
        JSON.stringify((this.board.properties as any).columns ?? [])
      );

      // Find the card in the source column
      const fromColumn = columns.find(c => c.id === fromColumnId);
      const toColumn = columns.find(c => c.id === toColumnId);

      if (!fromColumn || !toColumn) return;

      const cardIndex = fromColumn.cards.findIndex(c => c.uuid === cardId);
      if (cardIndex === -1) return;

      // Remove from source
      const [card] = fromColumn.cards.splice(cardIndex, 1);

      // Adjust target index if moving within the same column and moving down
      let adjustedIndex = toIndex;
      if (fromColumnId === toColumnId && cardIndex < toIndex) {
        adjustedIndex--;
      }

      // Insert into target
      toColumn.cards.splice(adjustedIndex, 0, card);

      // Update board properties
      const updatedProperties = {
        ...(this.board.properties as any),
        columns
      };

      // Save to database
      const sql = `
        UPDATE launchpad
        SET properties = CAST($1 AS JSONB)
        WHERE path = $2
      `;

      await db.executeSql(sql, [JSON.stringify(updatedProperties), this.board.path]);

      // Update local state
      (this.board.properties as any).columns = columns;

    } catch (error) {
      console.error('[BoardSync] Failed to save card move:', error);
      // Refresh board to restore correct state
      this.createColumnsFromBoard();
    }
  }

  getColumns(): Column3D[] {
    return Array.from(this.columns.values());
  }

  getColumn(id: string): Column3D | undefined {
    return this.columns.get(id);
  }

  dispose() {
    if (this.unsubscribe) {
      this.unsubscribe();
    }

    for (const column of this.columns.values()) {
      this.boardGroup.remove(column.group);
      column.dispose();
    }
    this.columns.clear();
  }
}

export function createBoardSync(
  board: PageNode,
  boardGroup: THREE.Group,
  config?: BoardSyncConfig
): BoardSync {
  return new BoardSync(board, boardGroup, config);
}
