<script lang="ts">
  import { LayoutGrid, Plus, ClipboardList } from 'lucide-svelte';
  import type { KanbanBoard } from './+page';

  interface Props {
    data: {
      boards: KanbanBoard[];
    };
  }

  let { data }: Props = $props();

  function getBoardId(board: KanbanBoard): string {
    // Extract the board name from the path (last segment)
    const segments = board.path.split('/').filter(Boolean);
    return segments[segments.length - 1];
  }

  function getCardCount(board: KanbanBoard): number {
    return (board.properties.columns ?? []).reduce(
      (total, col) => total + (col.cards?.length ?? 0),
      0
    );
  }

  function getColumnCount(board: KanbanBoard): number {
    return (board.properties.columns ?? []).length;
  }
</script>

<div class="tasks-page">
  <header class="page-header">
    <div class="header-content">
      <h1>
        <LayoutGrid size={28} />
        Task Boards
      </h1>
      <p class="subtitle">Manage your kanban boards and tasks</p>
    </div>
  </header>

  <main class="boards-grid">
    {#if data.boards.length === 0}
      <div class="empty-state">
        <ClipboardList size={48} />
        <h2>No boards yet</h2>
        <p>Create a kanban board in the admin console to get started.</p>
      </div>
    {:else}
      {#each data.boards as board (board.id)}
        <a href="/tasks/{getBoardId(board)}" class="board-card">
          <div class="board-icon">
            <LayoutGrid size={24} />
          </div>
          <div class="board-info">
            <h2>{board.properties.title}</h2>
            {#if board.properties.description}
              <p class="description">{board.properties.description}</p>
            {/if}
            <div class="board-stats">
              <span>{getColumnCount(board)} columns</span>
              <span class="separator">-</span>
              <span>{getCardCount(board)} cards</span>
            </div>
          </div>
        </a>
      {/each}
    {/if}
  </main>
</div>

<style>
  .tasks-page {
    min-height: 100vh;
    padding: 2rem;
  }

  .page-header {
    max-width: 1200px;
    margin: 0 auto 2rem;
  }

  .header-content h1 {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    font-size: 2rem;
    font-weight: 700;
    font-family: var(--font-display);
    color: var(--color-text-heading);
    margin: 0 0 0.5rem;
  }

  .header-content h1 :global(svg) {
    color: var(--color-accent);
  }

  .subtitle {
    color: var(--color-text-muted);
    font-size: 1rem;
    margin: 0;
  }

  .boards-grid {
    max-width: 1200px;
    margin: 0 auto;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(320px, 1fr));
    gap: 1.5rem;
  }

  .empty-state {
    grid-column: 1 / -1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: 4rem 2rem;
    background: var(--color-bg-card);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-lg);
    color: var(--color-text-muted);
    text-align: center;
  }

  .empty-state h2 {
    margin: 1rem 0 0.5rem;
    font-size: 1.25rem;
    font-family: var(--font-display);
    color: var(--color-text-secondary);
  }

  .empty-state p {
    margin: 0;
    color: var(--color-text-muted);
  }

  .board-card {
    display: flex;
    align-items: flex-start;
    gap: 1rem;
    padding: 1.5rem;
    background: var(--color-bg-card);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-md);
    text-decoration: none;
    color: inherit;
    transition: transform 0.15s, border-color 0.2s, box-shadow 0.2s;
  }

  .board-card:hover {
    transform: translateY(-2px);
    border-color: var(--color-accent);
    box-shadow: 0 4px 20px rgba(212, 175, 55, 0.1);
  }

  .board-icon {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 48px;
    height: 48px;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-md);
    color: var(--color-accent);
    flex-shrink: 0;
    transition: border-color 0.2s, background 0.2s;
  }

  .board-card:hover .board-icon {
    border-color: var(--color-accent);
    background: var(--color-accent-muted);
  }

  .board-info {
    flex: 1;
    min-width: 0;
  }

  .board-info h2 {
    font-size: 1.125rem;
    font-weight: 600;
    font-family: var(--font-display);
    color: var(--color-text-heading);
    margin: 0 0 0.25rem;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .board-info .description {
    font-size: 0.875rem;
    color: var(--color-text-secondary);
    margin: 0 0 0.5rem;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  .board-stats {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-size: 0.75rem;
    color: var(--color-text-muted);
  }

  .separator {
    color: var(--color-border);
  }
</style>
