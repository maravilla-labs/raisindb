<script lang="ts">
  import { ChevronRight, Home } from 'lucide-svelte';

  interface Props {
    baseName: string;
    currentPath: string;
    onNavigate: (path: string) => void;
  }

  let { baseName, currentPath, onNavigate }: Props = $props();

  // Parse path into breadcrumb segments
  const segments = $derived(() => {
    if (!currentPath || currentPath === '/') return [];
    return currentPath.split('/').filter(Boolean).map((name, index, arr) => ({
      name,
      path: '/' + arr.slice(0, index + 1).join('/')
    }));
  });
</script>

<nav class="breadcrumbs" aria-label="Folder navigation">
  <ol>
    <li>
      <button
        class="breadcrumb-item root"
        onclick={() => onNavigate('')}
        aria-label="Go to root folder"
      >
        <Home size={16} />
        <span>{baseName}</span>
      </button>
    </li>
    {#each segments() as segment}
      <li>
        <ChevronRight size={16} class="separator" />
        <button
          class="breadcrumb-item"
          onclick={() => onNavigate(segment.path)}
        >
          {segment.name}
        </button>
      </li>
    {/each}
  </ol>
</nav>

<style>
  .breadcrumbs {
    margin-bottom: 1rem;
    background: var(--color-bg-card);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-md);
    padding: 0.75rem 1rem;
  }

  ol {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.25rem;
    list-style: none;
    margin: 0;
    padding: 0;
  }

  li {
    display: flex;
    align-items: center;
    gap: 0.25rem;
  }

  .breadcrumb-item {
    display: flex;
    align-items: center;
    gap: 0.375rem;
    padding: 0.375rem 0.625rem;
    background: transparent;
    border: none;
    border-radius: var(--radius-sm);
    font-size: 0.875rem;
    font-family: var(--font-body);
    color: var(--color-text-secondary);
    cursor: pointer;
    transition: background-color 0.15s, color 0.15s;
  }

  .breadcrumb-item:hover {
    background: var(--color-surface);
    color: var(--color-accent);
  }

  .breadcrumb-item.root {
    font-weight: 500;
    color: var(--color-text);
  }

  .breadcrumb-item.root:hover {
    color: var(--color-accent);
  }

  :global(.separator) {
    color: var(--color-text-muted);
    flex-shrink: 0;
  }
</style>
