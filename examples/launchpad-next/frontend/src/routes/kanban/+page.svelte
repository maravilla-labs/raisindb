<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { LayoutGrid, Plus, X, ArrowRight, RotateCcw } from 'lucide-svelte';
  import { kanbanStore, type KanbanCard, type KanbanColumn } from '$lib/stores/kanban';
  import KanbanFlowChat from '$lib/components/KanbanFlowChat.svelte';

  let board = $state(kanbanStore.getBoard());
  let flowChat = $state(kanbanStore.getActiveFlowChat());
  let addingToColumn = $state<string | null>(null);
  let newCardTitle = $state('');
  let newCardDesc = $state('');
  let unsub: (() => void) | null = null;

  onMount(() => {
    unsub = kanbanStore.subscribe(() => {
      board = kanbanStore.getBoard();
      flowChat = kanbanStore.getActiveFlowChat();
    });
  });

  onDestroy(() => { unsub?.(); });

  function handleMoveCard(card: KanbanCard, fromColumn: KanbanColumn, toColumnId: string) {
    kanbanStore.moveCard(card.id, fromColumn.id, toColumnId);
  }

  function handleAddCard(columnId: string) {
    if (!newCardTitle.trim()) return;
    kanbanStore.addCard(columnId, newCardTitle.trim(), newCardDesc.trim() || undefined);
    newCardTitle = '';
    newCardDesc = '';
    addingToColumn = null;
  }

  function getNextColumns(currentColumnId: string): KanbanColumn[] {
    return board.columns.filter(c => c.id !== currentColumnId);
  }

  async function handleChatSend(content: string) {
    await kanbanStore.sendChatMessage(content);
  }

  function handleChatDismiss() {
    kanbanStore.dismissFlowChat();
  }

  const colColors: Record<string, string> = { todo: '#3b82f6', 'in-progress': '#f59e0b', done: '#10b981' };
</script>

<div class="page">
  <header class="hdr">
    <div class="hdr-left">
      <LayoutGrid size={28} />
      <div>
        <h1>Kanban Board</h1>
        <p class="sub">Move a card to Done to trigger an AI flow</p>
      </div>
    </div>
    <button class="reset" onclick={() => kanbanStore.resetBoard()}>
      <RotateCcw size={16} /> Reset
    </button>
  </header>

  <div class="cols">
    {#each board.columns as column (column.id)}
      <div class="col">
        <div class="col-hdr">
          <span class="dot" style="background:{colColors[column.id] ?? '#6366f1'}"></span>
          <h2>{column.title}</h2>
          <span class="cnt">{column.cards.length}</span>
        </div>

        <div class="cards">
          {#each column.cards as card (card.id)}
            <div class="card">
              <div class="card-body">
                <h3>{card.title}</h3>
                {#if card.description}<p class="desc">{card.description}</p>{/if}
                {#if card.summary}
                  <div class="summary">
                    <span class="slabel">Summary</span>
                    <p>{card.summary}</p>
                  </div>
                {/if}
              </div>
              <div class="actions">
                {#each getNextColumns(column.id) as target (target.id)}
                  <button class="move" style="color:{colColors[target.id] ?? '#6366f1'}" onclick={() => handleMoveCard(card, column, target.id)}>
                    <ArrowRight size={14} /> {target.title}
                  </button>
                {/each}
                <button class="del" onclick={() => kanbanStore.deleteCard(column.id, card.id)}><X size={14} /></button>
              </div>
            </div>
          {/each}
        </div>

        {#if addingToColumn === column.id}
          <div class="add-form">
            <input type="text" placeholder="Card title..." bind:value={newCardTitle} onkeydown={(e) => e.key === 'Enter' && handleAddCard(column.id)} />
            <input type="text" placeholder="Description (optional)" bind:value={newCardDesc} onkeydown={(e) => e.key === 'Enter' && handleAddCard(column.id)} />
            <div class="add-btns">
              <button class="btn-add" onclick={() => handleAddCard(column.id)}>Add</button>
              <button class="btn-x" onclick={() => addingToColumn = null}>Cancel</button>
            </div>
          </div>
        {:else}
          <button class="add-card" onclick={() => { addingToColumn = column.id; newCardTitle = ''; newCardDesc = ''; }}>
            <Plus size={16} /> Add Card
          </button>
        {/if}
      </div>
    {/each}
  </div>

  {#if flowChat}
    <KanbanFlowChat
      cardTitle={flowChat.cardTitle}
      messages={flowChat.messages}
      isStreaming={flowChat.isStreaming}
      streamingText={flowChat.streamingText}
      error={flowChat.error}
      onSendMessage={handleChatSend}
      onDismiss={handleChatDismiss}
    />
  {/if}
</div>

<style>
  .page { min-height: calc(100vh - 140px); padding: 2rem; }
  .hdr { max-width: 1200px; margin: 0 auto 2rem; display: flex; align-items: flex-start; justify-content: space-between; }
  .hdr-left { display: flex; align-items: flex-start; gap: 0.75rem; color: var(--color-accent); }
  .hdr-left h1 { font-size: 1.75rem; font-weight: 700; font-family: var(--font-display); color: var(--color-text-heading); margin: 0; }
  .sub { color: var(--color-text-muted); font-size: 0.875rem; margin: 0.25rem 0 0; }
  .reset { display: flex; align-items: center; gap: 0.375rem; padding: 0.5rem 1rem; background: var(--color-bg-card); border: 1px solid var(--color-border); border-radius: var(--radius-sm); color: var(--color-text-secondary); font-size: 0.875rem; cursor: pointer; transition: border-color 0.2s, color 0.2s; }
  .reset:hover { border-color: var(--color-accent); color: var(--color-accent); }
  .cols { max-width: 1200px; margin: 0 auto; display: flex; gap: 1.5rem; overflow-x: auto; padding-bottom: 1rem; }
  .col { flex: 1; min-width: 280px; max-width: 380px; background: var(--color-bg-elevated); border: 1px solid var(--color-border); border-radius: var(--radius-md); padding: 1rem; display: flex; flex-direction: column; }
  .col-hdr { display: flex; align-items: center; gap: 0.5rem; margin-bottom: 1rem; padding: 0 0.25rem; }
  .dot { width: 10px; height: 10px; border-radius: 50%; flex-shrink: 0; }
  .col-hdr h2 { font-size: 0.875rem; font-weight: 600; color: var(--color-text-secondary); margin: 0; text-transform: uppercase; letter-spacing: 0.05em; flex: 1; }
  .cnt { background: var(--color-surface); color: var(--color-text-secondary); font-size: 0.75rem; font-weight: 600; padding: 0.125rem 0.5rem; border-radius: 9999px; border: 1px solid var(--color-border); }
  .cards { flex: 1; display: flex; flex-direction: column; gap: 0.75rem; min-height: 60px; }
  .card { background: var(--color-bg-card); border: 1px solid var(--color-border); border-radius: var(--radius-sm); padding: 0.75rem; transition: border-color 0.2s; }
  .card:hover { border-color: var(--color-text-muted); }
  .card-body h3 { font-size: 0.875rem; font-weight: 500; color: var(--color-text-heading); margin: 0 0 0.25rem; }
  .desc { font-size: 0.75rem; color: var(--color-text-secondary); margin: 0; line-height: 1.4; }
  .summary { margin-top: 0.5rem; padding: 0.5rem; background: var(--color-success-muted); border-radius: var(--radius-sm); border: 1px solid rgba(62, 207, 142, 0.2); }
  .slabel { font-size: 0.625rem; font-weight: 600; color: var(--color-success); text-transform: uppercase; letter-spacing: 0.05em; }
  .summary p { font-size: 0.75rem; color: var(--color-success); margin: 0.25rem 0 0; line-height: 1.4; }
  .actions { display: flex; align-items: center; gap: 0.375rem; margin-top: 0.5rem; flex-wrap: wrap; }
  .move { display: flex; align-items: center; gap: 0.25rem; padding: 0.25rem 0.5rem; background: var(--color-surface); border: 1px solid var(--color-border); border-radius: var(--radius-sm); font-size: 0.6875rem; color: var(--color-text-secondary); cursor: pointer; transition: border-color 0.2s, color 0.2s; }
  .move:hover { border-color: var(--color-text-muted); color: var(--color-text); }
  .del { display: flex; align-items: center; padding: 0.25rem; background: none; border: none; color: var(--color-text-muted); cursor: pointer; border-radius: 0.25rem; margin-left: auto; transition: color 0.2s; }
  .del:hover { color: var(--color-error); }
  .add-card { display: flex; align-items: center; gap: 0.5rem; width: 100%; padding: 0.75rem; margin-top: 0.75rem; background: transparent; border: none; border-radius: var(--radius-sm); color: var(--color-text-muted); font-size: 0.875rem; cursor: pointer; transition: background 0.2s, color 0.2s; }
  .add-card:hover { background: var(--color-surface); color: var(--color-text-secondary); }
  .add-form { margin-top: 0.75rem; background: var(--color-bg-card); border: 1px solid var(--color-border); border-radius: var(--radius-sm); padding: 0.75rem; display: flex; flex-direction: column; gap: 0.5rem; }
  .add-form input { width: 100%; padding: 0.5rem; background: var(--color-surface); border: 1px solid var(--color-border); border-radius: var(--radius-sm); font-size: 0.875rem; color: var(--color-text); font-family: var(--font-body); }
  .add-form input:focus { outline: none; border-color: var(--color-accent); box-shadow: 0 0 0 2px var(--color-accent-muted); }
  .add-btns { display: flex; gap: 0.5rem; }
  .btn-add { background: var(--color-accent); color: var(--color-bg); border: none; padding: 0.5rem 1rem; border-radius: var(--radius-sm); font-size: 0.875rem; font-weight: 600; cursor: pointer; transition: background 0.2s; }
  .btn-add:hover { background: var(--color-accent-hover); }
  .btn-x { background: transparent; color: var(--color-text-muted); border: none; padding: 0.5rem 1rem; font-size: 0.875rem; cursor: pointer; }
</style>
