<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { invalidateAll, goto } from '$app/navigation';
  import { GripVertical, Plus, X, RefreshCw, Trash2, ArrowLeft, AlertCircle } from 'lucide-svelte';
  import type { PageNode, Element } from '$lib/raisin';
  import { getDatabase, getClient } from '$lib/raisin';

  interface KanbanCard {
    uuid: string;
    element_type: string;
    title: string;
    description?: string;
  }

  interface KanbanColumn {
    id: string;
    title: string;
    cards: KanbanCard[];
  }

  interface Props {
    page: PageNode;
  }

  let { page }: Props = $props();

  // Local state for columns (mutable copy for drag-drop)
  let columns = $state<KanbanColumn[]>([]);

  // Real-time sync indicator
  let isSyncing = $state(false);
  let lastSyncTime = $state<Date | null>(null);

  // Track if we're currently saving to avoid update loops
  let isSaving = $state(false);

  // Delete confirmation
  let showDeleteConfirm = $state(false);
  let deleting = $state(false);

  // Deleted by another user notification
  let deletedByOther = $state(false);

  // Save error notification
  let saveError = $state<string | null>(null);

  // Initialize columns from page properties (with deduplication)
  $effect(() => {
    if (!isSaving) {
      const pageColumns = (page.properties as any).columns ?? [];
      // Deep clone and deduplicate cards to prevent duplicate key errors
      const clonedColumns: KanbanColumn[] = JSON.parse(JSON.stringify(pageColumns));

      // Deduplicate cards across all columns
      const seenUuids = new Set<string>();
      for (const col of clonedColumns) {
        col.cards = col.cards.filter(card => {
          if (seenUuids.has(card.uuid)) {
            console.warn('[kanban] Removing duplicate card:', card.uuid);
            return false;
          }
          seenUuids.add(card.uuid);
          return true;
        });
      }

      columns = clonedColumns;
    }
  });

  // Real-time subscription
  let unsubscribe: (() => void) | null = null;

  onMount(async () => {
    try {
      const client = getClient();
      const db = client.database('launchpad');
      const workspace = db.workspace('launchpad');
      const events = workspace.events();

      // Subscribe to updates and deletes on this specific node
      const subscription = await events.subscribe(
        {
          workspace: 'launchpad',
          path: page.path,
          event_types: ['node:updated', 'node:deleted'],
        },
        async (event) => {
          const eventData = event as any;
          const eventType = eventData.event_type;
          const nodeId = eventData.payload?.node_id;

          // Handle board deleted by another user
          if (eventType === 'node:deleted' && nodeId === page.id) {
            deletedByOther = true;
            // Navigate after showing message briefly
            setTimeout(() => {
              goto(getParentPath());
            }, 2500);
            return;
          }

          // Skip updates if we just saved ourselves
          if (isSaving) return;

          isSyncing = true;

          // Fetch the updated node data
          const sql = `
            SELECT properties
            FROM launchpad
            WHERE path = $1
            LIMIT 1
          `;
          const result = await db.executeSql(sql, [page.path]);
          if (result.rows && result.rows.length > 0) {
            const updatedColumns = (result.rows[0] as any).properties?.columns ?? [];
            columns = JSON.parse(JSON.stringify(updatedColumns));
            lastSyncTime = new Date();
          }

          setTimeout(() => {
            isSyncing = false;
          }, 500);
        }
      );

      unsubscribe = () => subscription.unsubscribe();
    } catch (error) {
      console.error('[kanban] Failed to subscribe:', error);
    }
  });

  onDestroy(() => {
    if (unsubscribe) {
      unsubscribe();
    }
  });

  // Drag state
  let draggedCard = $state<{ card: KanbanCard; fromColumnId: string; fromIndex: number } | null>(null);
  let dragOverColumnId = $state<string | null>(null);
  let dragOverCardIndex = $state<number | null>(null);

  // New card form state
  let addingToColumn = $state<string | null>(null);
  let newCardTitle = $state('');

  // Cross-window drag data format
  interface CrossBoardDragData {
    card: KanbanCard;
    sourceBoardPath: string;
    sourceBoardId: string;
    sourceColumnId: string;
  }

  const DRAG_DATA_TYPE = 'application/x-kanban-card';

  function handleDragStart(e: DragEvent, card: KanbanCard, columnId: string, cardIndex: number) {
    draggedCard = { card, fromColumnId: columnId, fromIndex: cardIndex };
    if (e.dataTransfer) {
      e.dataTransfer.effectAllowed = 'move';

      // Store full card data for cross-window drops
      const dragData: CrossBoardDragData = {
        card,
        sourceBoardPath: page.path,
        sourceBoardId: page.id,
        sourceColumnId: columnId,
      };
      e.dataTransfer.setData(DRAG_DATA_TYPE, JSON.stringify(dragData));
      e.dataTransfer.setData('text/plain', card.uuid); // Fallback
    }
  }

  function handleDragOverColumn(e: DragEvent, columnId: string) {
    e.preventDefault();
    if (e.dataTransfer) {
      e.dataTransfer.dropEffect = 'move';
    }
    dragOverColumnId = columnId;
    // When dragging over empty area of column, drop at end
    dragOverCardIndex = null;
  }

  function handleDragOverCard(e: DragEvent, columnId: string, cardIndex: number) {
    e.preventDefault();
    e.stopPropagation();
    if (e.dataTransfer) {
      e.dataTransfer.dropEffect = 'move';
    }
    dragOverColumnId = columnId;
    dragOverCardIndex = cardIndex;
  }

  function handleDragLeave(e: DragEvent) {
    // Only clear if leaving the column entirely
    const relatedTarget = e.relatedTarget as HTMLElement;
    if (!relatedTarget?.closest('.column')) {
      dragOverColumnId = null;
      dragOverCardIndex = null;
    }
  }

  async function handleDrop(e: DragEvent, toColumnId: string) {
    e.preventDefault();

    const toColumn = columns.find(c => c.id === toColumnId);
    if (!toColumn) {
      draggedCard = null;
      dragOverColumnId = null;
      dragOverCardIndex = null;
      return;
    }

    // Calculate target index
    let targetIndex = dragOverCardIndex !== null ? dragOverCardIndex : toColumn.cards.length;

    // Try to get cross-window drag data
    const crossWindowData = e.dataTransfer?.getData(DRAG_DATA_TYPE);

    if (crossWindowData) {
      try {
        const dragData: CrossBoardDragData = JSON.parse(crossWindowData);

        // Check if this is from a different window (draggedCard is null locally)
        // This works for both cross-board AND same-board-different-window scenarios
        const isFromDifferentWindow = !draggedCard;
        const isFromDifferentBoard = dragData.sourceBoardPath !== page.path;

        if (isFromDifferentWindow) {
          // Check if card already exists anywhere on this board (check ALL columns)
          let existingColumnIdx = -1;
          let existingCardIdx = -1;
          for (let i = 0; i < columns.length; i++) {
            const idx = columns[i].cards.findIndex(c => c.uuid === dragData.card.uuid);
            if (idx !== -1) {
              existingColumnIdx = i;
              existingCardIdx = idx;
              break;
            }
          }

          const cardExists = existingColumnIdx !== -1;

          if (cardExists) {
            // Card exists - remove from current position first
            columns[existingColumnIdx].cards.splice(existingCardIdx, 1);
          }

          // Add at new position
          const newCards = [...toColumn.cards];
          newCards.splice(targetIndex, 0, dragData.card);
          toColumn.cards = newCards;

          columns = [...columns];

          // Save this board
          await saveBoard();

          // If from a different board, also remove from source
          if (isFromDifferentBoard) {
            await removeCardFromSourceBoard(
              dragData.sourceBoardPath,
              dragData.sourceColumnId,
              dragData.card.uuid
            );
          }

          draggedCard = null;
          dragOverColumnId = null;
          dragOverCardIndex = null;
          return;
        }
      } catch (err) {
        console.warn('[kanban] Failed to parse cross-window data:', err);
      }
    }

    // Same-window drop (local drag state exists)
    if (!draggedCard) {
      dragOverColumnId = null;
      dragOverCardIndex = null;
      return;
    }

    const { card, fromColumnId, fromIndex } = draggedCard;
    const fromColumn = columns.find(c => c.id === fromColumnId);

    if (!fromColumn) {
      draggedCard = null;
      dragOverColumnId = null;
      dragOverCardIndex = null;
      return;
    }

    // Same column reorder
    if (fromColumnId === toColumnId) {
      if (fromIndex === targetIndex || fromIndex === targetIndex - 1) {
        // No change needed
        draggedCard = null;
        dragOverColumnId = null;
        dragOverCardIndex = null;
        return;
      }

      // Remove from old position
      const newCards = [...fromColumn.cards];
      newCards.splice(fromIndex, 1);

      // Adjust target index if we removed before it
      if (fromIndex < targetIndex) {
        targetIndex--;
      }

      // Insert at new position
      newCards.splice(targetIndex, 0, card);
      fromColumn.cards = newCards;
    } else {
      // Move between columns
      fromColumn.cards = fromColumn.cards.filter(c => c.uuid !== card.uuid);

      const newCards = [...toColumn.cards];
      newCards.splice(targetIndex, 0, card);
      toColumn.cards = newCards;
    }

    // Trigger reactivity
    columns = [...columns];

    draggedCard = null;
    dragOverColumnId = null;
    dragOverCardIndex = null;

    // Save to database
    await saveBoard();
  }

  function handleDragEnd() {
    draggedCard = null;
    dragOverColumnId = null;
    dragOverCardIndex = null;
  }

  async function addCard(columnId: string) {
    if (!newCardTitle.trim()) return;

    const column = columns.find(c => c.id === columnId);
    if (!column) return;

    const newCard: KanbanCard = {
      uuid: crypto.randomUUID(),
      element_type: 'launchpad:KanbanCard',
      title: newCardTitle.trim(),
      description: ''
    };

    column.cards = [...column.cards, newCard];
    columns = [...columns];

    newCardTitle = '';
    addingToColumn = null;

    await saveBoard();
  }

  async function deleteCard(columnId: string, cardUuid: string) {
    const column = columns.find(c => c.id === columnId);
    if (!column) return;

    column.cards = column.cards.filter(c => c.uuid !== cardUuid);
    columns = [...columns];

    await saveBoard();
  }

  async function saveBoard() {
    isSaving = true;
    try {
      const db = await getDatabase();

      // Build updated properties
      const updatedProperties = {
        ...(page.properties as any),
        columns: columns
      };

      // Update the node using SQL - cast JSON string to JSONB
      const sql = `
        UPDATE launchpad
        SET properties = CAST($1 AS JSONB)
        WHERE path = $2
      `;

      await db.executeSql(sql, [JSON.stringify(updatedProperties), page.path]);
      lastSyncTime = new Date();

      // Invalidate page data to re-fetch the updated board
      await invalidateAll();
    } catch (error) {
      console.error('[kanban] Failed to save board:', error);

      // Extract user-friendly error message
      const errorMessage = error instanceof Error ? error.message : String(error);
      // Extract validation error details if present
      const validationMatch = errorMessage.match(/Validation failed: (.+)/);
      saveError = validationMatch ? validationMatch[1] : errorMessage;

      // Auto-dismiss after 5 seconds
      setTimeout(() => {
        saveError = null;
      }, 5000);
    } finally {
      // Small delay before allowing subscription updates
      // This also allows the effect to resync columns from page.properties on error
      setTimeout(() => {
        isSaving = false;
      }, 500);
    }
  }

  /**
   * Remove a card from the source board (used for cross-board drag-drop).
   * Fetches the source board's properties, removes the card, and saves.
   */
  async function removeCardFromSourceBoard(
    sourceBoardPath: string,
    sourceColumnId: string,
    cardUuid: string
  ) {
    try {
      const db = await getDatabase();

      // Fetch the source board's current properties
      const selectSql = `
        SELECT properties
        FROM launchpad
        WHERE path = $1
        LIMIT 1
      `;
      const result = await db.executeSql(selectSql, [sourceBoardPath]);

      if (!result.rows || result.rows.length === 0) {
        console.error('[kanban] Source board not found:', sourceBoardPath);
        return;
      }

      const sourceProperties = (result.rows[0] as any).properties;
      const sourceColumns = sourceProperties?.columns ?? [];

      // Find the column and remove the card
      let cardRemoved = false;
      const updatedColumns = sourceColumns.map((col: KanbanColumn) => {
        if (col.id === sourceColumnId) {
          const filteredCards = col.cards.filter(c => c.uuid !== cardUuid);
          if (filteredCards.length !== col.cards.length) {
            cardRemoved = true;
          }
          return { ...col, cards: filteredCards };
        }
        return col;
      });

      if (!cardRemoved) {
        console.warn('[kanban] Card not found in source column, searching all columns');
        // Card might have been moved to a different column locally before cross-board drop
        for (const col of updatedColumns) {
          const idx = col.cards.findIndex((c: KanbanCard) => c.uuid === cardUuid);
          if (idx !== -1) {
            col.cards.splice(idx, 1);
            cardRemoved = true;
            break;
          }
        }
      }

      if (!cardRemoved) {
        console.warn('[kanban] Card was not found in source board');
        return;
      }

      // Update the source board
      const updatedProperties = {
        ...sourceProperties,
        columns: updatedColumns
      };

      const updateSql = `
        UPDATE launchpad
        SET properties = CAST($1 AS JSONB)
        WHERE path = $2
      `;

      await db.executeSql(updateSql, [JSON.stringify(updatedProperties), sourceBoardPath]);
    } catch (error) {
      console.error('[kanban] Failed to remove card from source board:', error);
    }
  }

  function startAddCard(columnId: string) {
    addingToColumn = columnId;
    newCardTitle = '';
  }

  function cancelAddCard() {
    addingToColumn = null;
    newCardTitle = '';
  }

  function getParentPath(): string {
    // Get parent path for navigation after delete
    const pathParts = page.path.split('/').filter(Boolean);
    // Remove workspace prefix and board name
    const parentParts = pathParts.slice(1, -1);
    return '/' + parentParts.join('/');
  }

  async function deleteBoard() {
    deleting = true;
    try {
      const db = await getDatabase();
      const sql = `DELETE FROM launchpad WHERE path = $1`;
      await db.executeSql(sql, [page.path]);

      // Navigate back to parent
      goto(getParentPath());
    } catch (error) {
      console.error('[kanban] Failed to delete board:', error);
    } finally {
      deleting = false;
      showDeleteConfirm = false;
    }
  }
</script>

<article class="kanban-board">
  <header class="board-header">
    <div class="header-left">
      <a href={getParentPath()} class="back-btn" title="Back to boards">
        <ArrowLeft size={20} />
      </a>
      <div class="header-content">
        <h1>{(page.properties as any).title}</h1>
        {#if (page.properties as any).description}
          <p class="description">{(page.properties as any).description}</p>
        {/if}
      </div>
    </div>
    <div class="header-right">
      <div class="sync-indicator" class:syncing={isSyncing}>
        <RefreshCw size={16} class={isSyncing ? 'spinning' : ''} />
        {#if lastSyncTime}
          <span class="sync-time">Synced</span>
        {/if}
      </div>
      <button class="delete-board-btn" onclick={() => showDeleteConfirm = true} title="Delete board">
        <Trash2 size={18} />
      </button>
    </div>
  </header>

  <!-- Deleted by another user notification -->
  {#if deletedByOther}
    <div class="deleted-notification">
      <div class="deleted-content">
        <Trash2 size={24} />
        <div>
          <p class="deleted-title">Board Deleted</p>
          <p class="deleted-message">This board was deleted by another user. Redirecting...</p>
        </div>
      </div>
    </div>
  {/if}

  <!-- Save error notification -->
  {#if saveError}
    <div class="error-notification">
      <div class="error-content">
        <AlertCircle size={24} />
        <div>
          <p class="error-title">Failed to save</p>
          <p class="error-message">{saveError}</p>
        </div>
        <button class="error-dismiss" onclick={() => saveError = null}>
          <X size={18} />
        </button>
      </div>
    </div>
  {/if}

  <!-- Delete Confirmation Modal -->
  {#if showDeleteConfirm}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="modal-overlay" onclick={() => showDeleteConfirm = false}>
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="modal" onclick={(e) => e.stopPropagation()}>
        <div class="modal-header">
          <h3>Delete Board</h3>
          <button class="modal-close" onclick={() => showDeleteConfirm = false}>
            <X size={20} />
          </button>
        </div>
        <div class="modal-body">
          <p class="delete-warning">
            Are you sure you want to delete <strong>{(page.properties as any).title}</strong>?
          </p>
          <p class="delete-info">This action cannot be undone. All cards in this board will be permanently deleted.</p>
        </div>
        <div class="modal-footer">
          <button class="btn-secondary" onclick={() => showDeleteConfirm = false} disabled={deleting}>
            Cancel
          </button>
          <button class="btn-danger" onclick={deleteBoard} disabled={deleting}>
            {#if deleting}
              Deleting...
            {:else}
              Delete Board
            {/if}
          </button>
        </div>
      </div>
    </div>
  {/if}

  <div class="columns-container" role="list">
    {#each columns as column (column.id)}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="column"
        class:drag-over={dragOverColumnId === column.id}
        ondragover={(e) => handleDragOverColumn(e, column.id)}
        ondragleave={handleDragLeave}
        ondrop={(e) => handleDrop(e, column.id)}
        role="listitem"
      >
        <div class="column-header">
          <h2>{column.title}</h2>
          <span class="card-count">{column.cards.length}</span>
        </div>

        <div class="cards-container" role="list">
          {#each column.cards as card, cardIndex (card.uuid)}
            <!-- Drop indicator before card -->
            {#if dragOverColumnId === column.id && dragOverCardIndex === cardIndex && draggedCard?.card.uuid !== card.uuid}
              <div class="drop-indicator"></div>
            {/if}
            <!-- svelte-ignore a11y_no_static_element_interactions -->
            <div
              class="card"
              class:dragging={draggedCard?.card.uuid === card.uuid}
              draggable="true"
              ondragstart={(e) => handleDragStart(e, card, column.id, cardIndex)}
              ondragover={(e) => handleDragOverCard(e, column.id, cardIndex)}
              ondragend={handleDragEnd}
              role="listitem"
            >
              <div class="card-drag-handle">
                <GripVertical size={16} />
              </div>
              <div class="card-content">
                <h3 class="card-title">{card.title}</h3>
                {#if card.description}
                  <p class="card-description">{card.description}</p>
                {/if}
              </div>
              <button
                class="card-delete"
                onclick={() => deleteCard(column.id, card.uuid)}
                aria-label="Delete card"
              >
                <X size={14} />
              </button>
            </div>
          {/each}
          <!-- Drop indicator at end of column -->
          {#if dragOverColumnId === column.id && dragOverCardIndex === null && draggedCard}
            <div class="drop-indicator"></div>
          {/if}
        </div>

        {#if addingToColumn === column.id}
          <div class="add-card-form">
            <input
              type="text"
              placeholder="Enter card title..."
              bind:value={newCardTitle}
              onkeydown={(e) => e.key === 'Enter' && addCard(column.id)}
            />
            <div class="add-card-actions">
              <button class="btn-add" onclick={() => addCard(column.id)}>Add</button>
              <button class="btn-cancel" onclick={cancelAddCard}>Cancel</button>
            </div>
          </div>
        {:else}
          <button class="add-card-btn" onclick={() => startAddCard(column.id)}>
            <Plus size={16} />
            Add Card
          </button>
        {/if}
      </div>
    {/each}
  </div>
</article>

<style>
  .kanban-board {
    min-height: 100vh;
    background: #f1f5f9;
    padding: 2rem;
  }

  .board-header {
    margin-bottom: 2rem;
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
  }

  .header-left {
    display: flex;
    align-items: flex-start;
    gap: 1rem;
  }

  .back-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 40px;
    height: 40px;
    background: white;
    border-radius: 0.5rem;
    color: #64748b;
    text-decoration: none;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.1);
    transition: color 0.2s, background-color 0.2s;
  }

  .back-btn:hover {
    color: #8b5cf6;
    background: #f8fafc;
  }

  .header-content h1 {
    font-size: 2rem;
    font-weight: 700;
    color: #1e293b;
    margin: 0 0 0.5rem;
  }

  .header-content .description {
    color: #64748b;
    font-size: 1rem;
    margin: 0;
  }

  .header-right {
    display: flex;
    align-items: center;
    gap: 0.75rem;
  }

  .delete-board-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 40px;
    height: 40px;
    background: white;
    border: none;
    border-radius: 0.5rem;
    color: #64748b;
    cursor: pointer;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.1);
    transition: color 0.2s, background-color 0.2s;
  }

  .delete-board-btn:hover {
    color: #ef4444;
    background: #fef2f2;
  }

  .sync-indicator {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    color: #94a3b8;
    font-size: 0.75rem;
    padding: 0.5rem 0.75rem;
    background: white;
    border-radius: 0.5rem;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.1);
  }

  .sync-indicator.syncing {
    color: #8b5cf6;
  }

  .sync-indicator :global(.spinning) {
    animation: spin 1s linear infinite;
  }

  @keyframes spin {
    from { transform: rotate(0deg); }
    to { transform: rotate(360deg); }
  }

  .sync-time {
    color: #10b981;
  }

  .columns-container {
    display: flex;
    gap: 1.5rem;
    overflow-x: auto;
    padding-bottom: 1rem;
  }

  .column {
    flex: 0 0 320px;
    background: #e2e8f0;
    border-radius: 0.75rem;
    padding: 1rem;
    display: flex;
    flex-direction: column;
    max-height: calc(100vh - 200px);
    transition: background-color 0.2s;
  }

  .column.drag-over {
    background: #cbd5e1;
  }

  .column-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 1rem;
    padding: 0 0.25rem;
  }

  .column-header h2 {
    font-size: 0.875rem;
    font-weight: 600;
    color: #475569;
    margin: 0;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .card-count {
    background: #94a3b8;
    color: white;
    font-size: 0.75rem;
    font-weight: 600;
    padding: 0.125rem 0.5rem;
    border-radius: 9999px;
  }

  .cards-container {
    flex: 1;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    min-height: 100px;
  }

  .card {
    background: white;
    border-radius: 0.5rem;
    padding: 0.75rem;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.1);
    cursor: grab;
    display: flex;
    align-items: flex-start;
    gap: 0.5rem;
    transition: transform 0.15s, box-shadow 0.15s;
    position: relative;
  }

  .card:hover {
    box-shadow: 0 4px 6px rgba(0, 0, 0, 0.1);
  }

  .card:active {
    cursor: grabbing;
    transform: rotate(2deg);
  }

  .card.dragging {
    opacity: 0.5;
    transform: scale(0.98);
  }

  .drop-indicator {
    height: 4px;
    background: #8b5cf6;
    border-radius: 2px;
    margin: -2px 0;
    animation: pulse 1s ease-in-out infinite;
  }

  @keyframes pulse {
    0%, 100% { opacity: 0.6; }
    50% { opacity: 1; }
  }

  .card-drag-handle {
    color: #94a3b8;
    flex-shrink: 0;
    padding-top: 0.125rem;
  }

  .card-content {
    flex: 1;
    min-width: 0;
  }

  .card-title {
    font-size: 0.875rem;
    font-weight: 500;
    color: #1e293b;
    margin: 0 0 0.25rem;
    word-wrap: break-word;
  }

  .card-description {
    font-size: 0.75rem;
    color: #64748b;
    margin: 0;
    line-height: 1.4;
  }

  .card-delete {
    position: absolute;
    top: 0.5rem;
    right: 0.5rem;
    background: none;
    border: none;
    color: #94a3b8;
    cursor: pointer;
    padding: 0.25rem;
    border-radius: 0.25rem;
    opacity: 0;
    transition: opacity 0.15s, color 0.15s;
  }

  .card:hover .card-delete {
    opacity: 1;
  }

  .card-delete:hover {
    color: #ef4444;
    background: #fef2f2;
  }

  .add-card-btn {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    width: 100%;
    padding: 0.75rem;
    margin-top: 0.75rem;
    background: transparent;
    border: none;
    border-radius: 0.5rem;
    color: #64748b;
    font-size: 0.875rem;
    cursor: pointer;
    transition: background-color 0.15s, color 0.15s;
  }

  .add-card-btn:hover {
    background: #cbd5e1;
    color: #475569;
  }

  .add-card-form {
    margin-top: 0.75rem;
    background: white;
    border-radius: 0.5rem;
    padding: 0.75rem;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.1);
  }

  .add-card-form input {
    width: 100%;
    padding: 0.5rem;
    border: 1px solid #e2e8f0;
    border-radius: 0.25rem;
    font-size: 0.875rem;
    margin-bottom: 0.5rem;
  }

  .add-card-form input:focus {
    outline: none;
    border-color: #8b5cf6;
    box-shadow: 0 0 0 2px rgba(139, 92, 246, 0.2);
  }

  .add-card-actions {
    display: flex;
    gap: 0.5rem;
  }

  .btn-add {
    background: #8b5cf6;
    color: white;
    border: none;
    padding: 0.5rem 1rem;
    border-radius: 0.25rem;
    font-size: 0.875rem;
    cursor: pointer;
    transition: background-color 0.15s;
  }

  .btn-add:hover {
    background: #7c3aed;
  }

  .btn-cancel {
    background: transparent;
    color: #64748b;
    border: none;
    padding: 0.5rem 1rem;
    font-size: 0.875rem;
    cursor: pointer;
  }

  .btn-cancel:hover {
    color: #475569;
  }

  /* Deleted notification */
  .deleted-notification {
    position: fixed;
    top: 2rem;
    left: 50%;
    transform: translateX(-50%);
    z-index: 1100;
    background: #fef2f2;
    border: 1px solid #fecaca;
    border-radius: 0.75rem;
    padding: 1rem 1.5rem;
    box-shadow: 0 10px 25px rgba(0, 0, 0, 0.15);
    animation: slide-down 0.3s ease-out;
  }

  @keyframes slide-down {
    from {
      opacity: 0;
      transform: translateX(-50%) translateY(-1rem);
    }
    to {
      opacity: 1;
      transform: translateX(-50%) translateY(0);
    }
  }

  .deleted-content {
    display: flex;
    align-items: center;
    gap: 1rem;
    color: #dc2626;
  }

  .deleted-title {
    font-weight: 600;
    font-size: 1rem;
    margin: 0 0 0.25rem;
    color: #991b1b;
  }

  .deleted-message {
    font-size: 0.875rem;
    margin: 0;
    color: #b91c1c;
  }

  /* Error notification */
  .error-notification {
    position: fixed;
    top: 2rem;
    left: 50%;
    transform: translateX(-50%);
    z-index: 1100;
    background: #fef2f2;
    border: 1px solid #fecaca;
    border-radius: 0.75rem;
    padding: 1rem 1.5rem;
    box-shadow: 0 10px 25px rgba(0, 0, 0, 0.15);
    animation: slide-down 0.3s ease-out;
    max-width: 90vw;
  }

  .error-content {
    display: flex;
    align-items: flex-start;
    gap: 1rem;
    color: #dc2626;
  }

  .error-title {
    font-weight: 600;
    font-size: 1rem;
    margin: 0 0 0.25rem;
    color: #991b1b;
  }

  .error-message {
    font-size: 0.875rem;
    margin: 0;
    color: #b91c1c;
    word-break: break-word;
  }

  .error-dismiss {
    background: none;
    border: none;
    color: #991b1b;
    cursor: pointer;
    padding: 0.25rem;
    border-radius: 0.25rem;
    flex-shrink: 0;
  }

  .error-dismiss:hover {
    background: #fecaca;
  }

  /* Modal styles */
  .modal-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.5);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1000;
    padding: 1rem;
  }

  .modal {
    background: white;
    border-radius: 1rem;
    width: 100%;
    max-width: 480px;
    box-shadow: 0 20px 40px rgba(0, 0, 0, 0.2);
  }

  .modal-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 1.5rem;
    border-bottom: 1px solid #fecaca;
  }

  .modal-header h3 {
    font-size: 1.25rem;
    font-weight: 600;
    margin: 0;
    color: #1e293b;
  }

  .modal-close {
    background: none;
    border: none;
    color: #64748b;
    cursor: pointer;
    padding: 0.25rem;
    border-radius: 0.25rem;
    transition: color 0.15s;
  }

  .modal-close:hover {
    color: #1e293b;
  }

  .modal-body {
    padding: 1.5rem;
  }

  .delete-warning {
    font-size: 1rem;
    color: #1e293b;
    margin: 0 0 0.75rem;
  }

  .delete-info {
    font-size: 0.875rem;
    color: #64748b;
    margin: 0;
  }

  .modal-footer {
    display: flex;
    justify-content: flex-end;
    gap: 0.75rem;
    padding: 1.5rem;
    border-top: 1px solid #e2e8f0;
    background: #f8fafc;
    border-radius: 0 0 1rem 1rem;
  }

  .btn-secondary {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.625rem 1.25rem;
    border-radius: 0.5rem;
    font-size: 0.875rem;
    font-weight: 500;
    cursor: pointer;
    transition: background-color 0.15s, opacity 0.15s;
    background: white;
    color: #374151;
    border: 1px solid #e2e8f0;
  }

  .btn-secondary:hover:not(:disabled) {
    background: #f1f5f9;
  }

  .btn-secondary:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  .btn-danger {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.625rem 1.25rem;
    border-radius: 0.5rem;
    font-size: 0.875rem;
    font-weight: 500;
    cursor: pointer;
    transition: background-color 0.15s, opacity 0.15s;
    background: #ef4444;
    color: white;
    border: none;
  }

  .btn-danger:hover:not(:disabled) {
    background: #dc2626;
  }

  .btn-danger:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }
</style>
