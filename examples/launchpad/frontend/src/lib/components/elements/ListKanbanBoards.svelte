<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { goto } from '$app/navigation';
  import { flip } from 'svelte/animate';
  import { scale, fade } from 'svelte/transition';
  import { tweened } from 'svelte/motion';
  import { cubicOut } from 'svelte/easing';
  import { LayoutGrid, ArrowRight, Plus, X, Trash2 } from 'lucide-svelte';
  import { query, getDatabase, getClient, type PageNode, type Element } from '$lib/raisin';
  import { page } from '$app/stores';
  import { currentAction, clearAction } from '$lib/stores/actions';

  interface ListKanbanBoardsElement extends Element {
    heading?: string;
  }

  interface Props {
    element: ListKanbanBoardsElement;
  }

  let { element }: Props = $props();

  let boards = $state<PageNode[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);

  // Create board form state
  let showCreateForm = $state(false);
  let newBoardTitle = $state('');
  let newBoardDescription = $state('');
  let creating = $state(false);

  // Delete confirmation state
  let showDeleteConfirm = $state(false);
  let boardToDelete = $state<PageNode | null>(null);
  let deleting = $state(false);

  // Store current path for board creation
  let currentParentPath = $state('');

  // Simple set of recently updated board IDs (for pulse animation)
  let recentlyUpdatedIds = $state<Set<string>>(new Set());

  // Animated number values (per board id)
  let animatedCardCounts = $state<Map<string, number>>(new Map());
  let animatedColumnCounts = $state<Map<string, number>>(new Map());

  // Store tweened instances (not reactive, just for animation)
  const tweenedStores = new Map<string, { cards: ReturnType<typeof tweened<number>>, columns: ReturnType<typeof tweened<number>> }>();

  // Listen for actions from other components (e.g., Hero)
  let unsubscribeAction: (() => void) | null = null;

  // Real-time subscription cleanup
  let unsubscribeEvents: (() => void) | null = null;

  // Initialize tweened stores when boards change
  $effect(() => {
    for (const board of boards) {
      const cardCount = getCardCount(board);
      const columnCount = getColumnCount(board);
      ensureTweenedStores(board.id, cardCount, columnCount);
    }
  });

  onMount(async () => {
    await loadBoards();
    await setupSubscription();

    // Subscribe to actions
    unsubscribeAction = currentAction.subscribe((action) => {
      if (action === 'createBoard') {
        showCreateForm = true;
        clearAction();
      }
    });
  });

  onDestroy(() => {
    if (unsubscribeAction) {
      unsubscribeAction();
    }
    if (unsubscribeEvents) {
      unsubscribeEvents();
    }
  });

  async function setupSubscription() {
    try {
      const client = getClient();
      const db = client.database('launchpad');
      const workspace = db.workspace('launchpad');
      const events = workspace.events();

      // Subscribe to node creation and updates in child paths
      // Use wildcard pattern to match children: /launchpad/tasks/*
      const parentNodePath = `/launchpad${currentParentPath}/*`;

      const subscription = await events.subscribe(
        {
          workspace: 'launchpad',
          path: parentNodePath,
          event_types: ['node:created', 'node:updated', 'node:deleted'],
        },
        async (event) => {
          const eventData = event as any;
          const nodeId = eventData.payload?.node_id;
          const eventType = eventData.event_type;

          // For updates, add to recently updated set
          if (nodeId && eventType === 'node:updated') {
            recentlyUpdatedIds = new Set([...recentlyUpdatedIds, nodeId]);

            // Remove after 3 seconds
            setTimeout(() => {
              recentlyUpdatedIds = new Set([...recentlyUpdatedIds].filter(id => id !== nodeId));
            }, 3000);
          }

          // Reload boards list
          await loadBoards();
        }
      );

      unsubscribeEvents = () => {
        subscription.unsubscribe();
      };
    } catch (e) {
      console.error('[ListKanbanBoards] Failed to setup subscription:', e);
    }
  }

  async function loadBoards() {
    try {
      // Get the current page path to find sibling boards
      const currentPath = $page.url.pathname;
      currentParentPath = currentPath.endsWith('/') ? currentPath.slice(0, -1) : currentPath;

      // Query for child pages with KanbanBoard archetype
      const sql = `
        SELECT id, path, name, node_type, archetype, properties
        FROM launchpad
        WHERE CHILD_OF('/launchpad${currentParentPath}')
          AND archetype = 'launchpad:KanbanBoard'
      `;

      const results = await query<PageNode>(sql);
      boards = results;

      // Update tweened counts for smooth number animations
      updateTweenedCounts();
    } catch (e) {
      console.error('[ListKanbanBoards] Failed to load boards:', e);
      error = e instanceof Error ? e.message : 'Failed to load boards';
    } finally {
      loading = false;
    }
  }

  function getBoardUrl(board: PageNode): string {
    // Extract the path after /launchpad/
    const pathParts = board.path.split('/').filter(Boolean);
    // Remove 'launchpad' workspace prefix
    const slug = pathParts.slice(1).join('/');
    return `/${slug}`;
  }

  function getCardCount(board: PageNode): number {
    const columns = (board.properties as any).columns ?? [];
    return columns.reduce((total: number, col: any) => total + (col.cards?.length ?? 0), 0);
  }

  function getColumnCount(board: PageNode): number {
    return ((board.properties as any).columns ?? []).length;
  }

  // Get or create tweened stores for a board and set up subscriptions
  function ensureTweenedStores(boardId: string, cardCount: number, columnCount: number) {
    if (!tweenedStores.has(boardId)) {
      const cardStore = tweened(cardCount, { duration: 1200, easing: cubicOut });
      const columnStore = tweened(columnCount, { duration: 1200, easing: cubicOut });

      // Subscribe to update reactive state
      cardStore.subscribe(value => {
        animatedCardCounts.set(boardId, value);
        animatedCardCounts = new Map(animatedCardCounts);
      });
      columnStore.subscribe(value => {
        animatedColumnCounts.set(boardId, value);
        animatedColumnCounts = new Map(animatedColumnCounts);
      });

      tweenedStores.set(boardId, { cards: cardStore, columns: columnStore });
    }
    return tweenedStores.get(boardId)!;
  }

  // Update tweened values for all boards
  function updateTweenedCounts() {
    for (const board of boards) {
      const cardCount = getCardCount(board);
      const columnCount = getColumnCount(board);

      const stores = ensureTweenedStores(board.id, cardCount, columnCount);
      stores.cards.set(cardCount);
      stores.columns.set(columnCount);
    }
  }

  // Get animated card count for a board
  function getAnimatedCardCount(boardId: string): number {
    return animatedCardCounts.get(boardId) ?? 0;
  }

  // Get animated column count for a board
  function getAnimatedColumnCount(boardId: string): number {
    return animatedColumnCounts.get(boardId) ?? 0;
  }

  function slugify(text: string): string {
    return text
      .toLowerCase()
      .trim()
      .replace(/[^\w\s-]/g, '')
      .replace(/[\s_-]+/g, '-')
      .replace(/^-+|-+$/g, '');
  }

  async function createBoard() {
    if (!newBoardTitle.trim()) return;

    creating = true;
    try {
      const db = await getDatabase();
      const slug = slugify(newBoardTitle);
      const parentNodePath = `/launchpad${currentParentPath}`;

      // Default columns for a new board
      const defaultColumns = [
        { id: 'col-backlog', title: 'Backlog', cards: [] },
        { id: 'col-in-progress', title: 'In Progress', cards: [] },
        { id: 'col-done', title: 'Done', cards: [] }
      ];

      const properties = {
        title: newBoardTitle.trim(),
        slug: slug,
        description: newBoardDescription.trim() || null,
        columns: defaultColumns
      };

      // Insert new board using SQL
      const sql = `
        INSERT INTO launchpad (path, name, node_type, archetype, properties)
        VALUES ($1, $2, $3, $4, CAST($5 AS JSONB))
      `;

      const newPath = `${parentNodePath}/${slug}`;
      await db.executeSql(sql, [
        newPath,
        slug,
        'launchpad:Page',
        'launchpad:KanbanBoard',
        JSON.stringify(properties)
      ]);

      // Reset form
      showCreateForm = false;
      newBoardTitle = '';
      newBoardDescription = '';

      // Navigate to the new board
      goto(`${currentParentPath}/${slug}`);
    } catch (e) {
      console.error('[ListKanbanBoards] Failed to create board:', e);
      error = e instanceof Error ? e.message : 'Failed to create board';
    } finally {
      creating = false;
    }
  }

  function cancelCreate() {
    showCreateForm = false;
    newBoardTitle = '';
    newBoardDescription = '';
  }

  function confirmDelete(board: PageNode, event: MouseEvent) {
    event.preventDefault();
    event.stopPropagation();
    boardToDelete = board;
    showDeleteConfirm = true;
  }

  function cancelDelete() {
    showDeleteConfirm = false;
    boardToDelete = null;
  }

  async function deleteBoard() {
    if (!boardToDelete) return;

    deleting = true;
    try {
      const db = await getDatabase();
      const sql = `DELETE FROM launchpad WHERE path = $1`;
      await db.executeSql(sql, [boardToDelete.path]);

      // Close modal and reload
      showDeleteConfirm = false;
      boardToDelete = null;
      await loadBoards();
    } catch (e) {
      console.error('[ListKanbanBoards] Failed to delete board:', e);
      error = e instanceof Error ? e.message : 'Failed to delete board';
    } finally {
      deleting = false;
    }
  }
</script>

<section class="list-kanban-boards">
  <div class="container">
    <div class="section-header">
      {#if element.heading}
        <h2 class="heading">{element.heading}</h2>
      {/if}
      <button class="create-btn" onclick={() => showCreateForm = true}>
        <Plus size={18} />
        Create Board
      </button>
    </div>

    <!-- Create Board Form Modal -->
    {#if showCreateForm}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="modal-overlay" onclick={cancelCreate}>
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div class="modal" onclick={(e) => e.stopPropagation()}>
          <div class="modal-header">
            <h3>Create New Board</h3>
            <button class="modal-close" onclick={cancelCreate}>
              <X size={20} />
            </button>
          </div>
          <div class="modal-body">
            <div class="form-group">
              <label for="board-title">Board Title</label>
              <input
                id="board-title"
                type="text"
                placeholder="e.g., Sprint Board, Project Tasks"
                bind:value={newBoardTitle}
                onkeydown={(e) => e.key === 'Enter' && createBoard()}
              />
            </div>
            <div class="form-group">
              <label for="board-description">Description (optional)</label>
              <textarea
                id="board-description"
                placeholder="What is this board for?"
                bind:value={newBoardDescription}
                rows="3"
              ></textarea>
            </div>
          </div>
          <div class="modal-footer">
            <button class="btn-secondary" onclick={cancelCreate} disabled={creating}>
              Cancel
            </button>
            <button
              class="btn-primary"
              onclick={createBoard}
              disabled={!newBoardTitle.trim() || creating}
            >
              {#if creating}
                Creating...
              {:else}
                Create Board
              {/if}
            </button>
          </div>
        </div>
      </div>
    {/if}

    {#if loading}
      <div class="loading">
        <div class="spinner"></div>
        <span>Loading boards...</span>
      </div>
    {:else if error}
      <div class="error">
        <p>{error}</p>
        <button class="btn-secondary" onclick={() => { error = null; }}>Dismiss</button>
      </div>
    {:else if boards.length === 0}
      <div class="empty">
        <LayoutGrid size={48} />
        <h3>No boards yet</h3>
        <p>Create your first Kanban board to get started</p>
        <button class="btn-primary" onclick={() => showCreateForm = true}>
          <Plus size={18} />
          Create Board
        </button>
      </div>
    {:else}
      <div class="boards-grid">
        {#each boards as board (board.id)}
          {@const isUpdated = recentlyUpdatedIds.has(board.id)}
          <a
            href={getBoardUrl(board)}
            class="board-card"
            class:is-updated={isUpdated}
            animate:flip={{ duration: 300 }}
            in:scale={{ duration: 300, start: 0.9 }}
          >
            <div class="board-icon">
              <LayoutGrid size={24} />
            </div>
            <div class="board-info">
              <div class="board-title-row">
                <h3 class="board-title">{(board.properties as any).title}</h3>
                {#if isUpdated}
                  <span class="update-badge" in:scale={{ duration: 200 }}>UPDATED</span>
                {/if}
              </div>
              {#if (board.properties as any).description}
                <p class="board-description">{(board.properties as any).description}</p>
              {/if}
              <div class="board-stats">
                <span class="stat">
                  <span class="stat-number">{Math.round(getAnimatedColumnCount(board.id))}</span> columns
                </span>
                <span class="stat">
                  <span class="stat-number">{Math.round(getAnimatedCardCount(board.id))}</span> cards
                </span>
              </div>
            </div>
            <div class="board-actions">
              <button
                class="delete-btn"
                onclick={(e) => confirmDelete(board, e)}
                title="Delete board"
              >
                <Trash2 size={16} />
              </button>
              <div class="board-arrow">
                <ArrowRight size={20} />
              </div>
            </div>
          </a>
        {/each}
      </div>
    {/if}

    <!-- Delete Confirmation Modal -->
    {#if showDeleteConfirm && boardToDelete}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="modal-overlay" onclick={cancelDelete}>
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div class="modal modal-delete" onclick={(e) => e.stopPropagation()}>
          <div class="modal-header">
            <h3>Delete Board</h3>
            <button class="modal-close" onclick={cancelDelete}>
              <X size={20} />
            </button>
          </div>
          <div class="modal-body">
            <p class="delete-warning">
              Are you sure you want to delete <strong>{(boardToDelete.properties as any).title}</strong>?
            </p>
            <p class="delete-info">This action cannot be undone. All cards in this board will be permanently deleted.</p>
          </div>
          <div class="modal-footer">
            <button class="btn-secondary" onclick={cancelDelete} disabled={deleting}>
              Cancel
            </button>
            <button
              class="btn-danger"
              onclick={deleteBoard}
              disabled={deleting}
            >
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
  </div>
</section>

<style>
  .list-kanban-boards {
    padding: 4rem 2rem;
    background: white;
  }

  .container {
    max-width: 1200px;
    margin: 0 auto;
  }

  .section-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 2.5rem;
  }

  .heading {
    font-size: 1.75rem;
    font-weight: 600;
    margin: 0;
    color: #1e293b;
  }

  .create-btn {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.75rem 1.25rem;
    background: #8b5cf6;
    color: white;
    border: none;
    border-radius: 0.5rem;
    font-size: 0.875rem;
    font-weight: 500;
    cursor: pointer;
    transition: background-color 0.15s;
  }

  .create-btn:hover {
    background: #7c3aed;
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
    border-bottom: 1px solid #e2e8f0;
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

  .form-group {
    margin-bottom: 1.25rem;
  }

  .form-group:last-child {
    margin-bottom: 0;
  }

  .form-group label {
    display: block;
    font-size: 0.875rem;
    font-weight: 500;
    color: #374151;
    margin-bottom: 0.5rem;
  }

  .form-group input,
  .form-group textarea {
    width: 100%;
    padding: 0.75rem;
    border: 1px solid #e2e8f0;
    border-radius: 0.5rem;
    font-size: 0.875rem;
    transition: border-color 0.15s, box-shadow 0.15s;
  }

  .form-group input:focus,
  .form-group textarea:focus {
    outline: none;
    border-color: #8b5cf6;
    box-shadow: 0 0 0 3px rgba(139, 92, 246, 0.15);
  }

  .form-group textarea {
    resize: vertical;
    min-height: 80px;
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

  .btn-primary,
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
  }

  .btn-primary {
    background: #8b5cf6;
    color: white;
    border: none;
  }

  .btn-primary:hover:not(:disabled) {
    background: #7c3aed;
  }

  .btn-primary:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  .btn-secondary {
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

  .loading {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 1rem;
    padding: 3rem;
    color: #64748b;
  }

  .spinner {
    width: 32px;
    height: 32px;
    border: 3px solid #e2e8f0;
    border-top-color: #8b5cf6;
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .error {
    text-align: center;
    padding: 2rem;
    background: #fef2f2;
    border-radius: 0.75rem;
    color: #991b1b;
  }

  .empty {
    text-align: center;
    padding: 4rem 2rem;
    color: #64748b;
  }

  .empty h3 {
    font-size: 1.25rem;
    font-weight: 600;
    margin: 1rem 0 0.5rem;
    color: #475569;
  }

  .empty p {
    margin: 0;
  }

  .boards-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(320px, 1fr));
    gap: 1.5rem;
  }

  .board-card {
    display: flex;
    align-items: center;
    gap: 1rem;
    padding: 1.5rem;
    background: #f8fafc;
    border: 1px solid #e2e8f0;
    border-radius: 0.75rem;
    text-decoration: none;
    color: inherit;
    transition: transform 0.2s, box-shadow 0.2s, border-color 0.2s;
  }

  .board-card:hover {
    transform: translateY(-2px);
    box-shadow: 0 8px 16px rgba(0, 0, 0, 0.1);
    border-color: #8b5cf6;
  }

  .board-card.is-updated {
    animation: pulse-update 0.6s ease-out;
  }

  @keyframes pulse-update {
    0% { transform: scale(1); }
    50% { transform: scale(1.02); }
    100% { transform: scale(1); }
  }

  .board-icon {
    flex-shrink: 0;
    width: 48px;
    height: 48px;
    background: linear-gradient(135deg, #8b5cf6 0%, #a78bfa 100%);
    border-radius: 0.75rem;
    display: flex;
    align-items: center;
    justify-content: center;
    color: white;
  }

  .board-info {
    flex: 1;
    min-width: 0;
  }

  .board-title-row {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin-bottom: 0.25rem;
  }

  .board-title {
    font-size: 1.125rem;
    font-weight: 600;
    margin: 0;
    color: #1e293b;
  }

  .update-badge {
    font-size: 0.625rem;
    font-weight: 700;
    padding: 0.125rem 0.5rem;
    background: #8b5cf6;
    color: white;
    border-radius: 0.25rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    animation: badge-pulse 1s ease-in-out infinite;
  }

  @keyframes badge-pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.7; }
  }

  .board-description {
    font-size: 0.875rem;
    color: #64748b;
    margin: 0 0 0.75rem;
    line-height: 1.4;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  .board-stats {
    display: flex;
    gap: 1rem;
    font-size: 0.75rem;
    color: #94a3b8;
  }

  .board-stats .stat {
    display: flex;
    align-items: center;
    gap: 0.25rem;
  }

  .stat-number {
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    min-width: 1.25em;
    text-align: right;
    transition: color 0.3s, transform 0.3s;
  }

  .board-card.is-updated .stat-number {
    color: #8b5cf6;
    animation: number-pop 0.4s ease-out;
  }

  @keyframes number-pop {
    0% { transform: scale(1); }
    50% { transform: scale(1.3); }
    100% { transform: scale(1); }
  }

  .board-actions {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    flex-shrink: 0;
  }

  .delete-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 32px;
    height: 32px;
    background: transparent;
    border: none;
    border-radius: 0.375rem;
    color: #94a3b8;
    cursor: pointer;
    opacity: 0;
    transition: opacity 0.2s, color 0.2s, background-color 0.2s;
  }

  .board-card:hover .delete-btn {
    opacity: 1;
  }

  .delete-btn:hover {
    color: #ef4444;
    background: #fef2f2;
  }

  .board-arrow {
    flex-shrink: 0;
    color: #94a3b8;
    transition: color 0.2s, transform 0.2s;
  }

  .board-card:hover .board-arrow {
    color: #8b5cf6;
    transform: translateX(4px);
  }

  /* Delete modal styles */
  .modal-delete .modal-header {
    border-bottom-color: #fecaca;
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
