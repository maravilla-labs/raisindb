<script lang="ts">
  import { Code, Copy, Check, ChevronDown, ChevronRight } from 'lucide-svelte';

  let { sql, title = "SQL Query" }: { sql: string; title?: string } = $props();
  let expanded = $state(false);
  let copied = $state(false);

  async function copyToClipboard() {
    try {
      await navigator.clipboard.writeText(sql);
      copied = true;
      setTimeout(() => copied = false, 2000);
    } catch (err) {
      console.error('Failed to copy:', err);
    }
  }
</script>

<div class="sql-display">
  <button class="toggle" onclick={() => expanded = !expanded}>
    {#if expanded}
      <ChevronDown size={16} />
    {:else}
      <ChevronRight size={16} />
    {/if}
    <Code size={16} />
    <span>{expanded ? 'Hide' : 'Show'} SQL</span>
  </button>

  {#if expanded}
    <div class="sql-panel">
      <div class="sql-header">
        <span class="sql-title">{title}</span>
        <button class="copy-btn" onclick={copyToClipboard} title="Copy SQL">
          {#if copied}
            <Check size={14} />
            <span>Copied!</span>
          {:else}
            <Copy size={14} />
            <span>Copy</span>
          {/if}
        </button>
      </div>
      <pre><code>{sql.trim()}</code></pre>
    </div>
  {/if}
</div>

<style>
  .sql-display {
    margin-bottom: 1rem;
  }

  .toggle {
    display: inline-flex;
    align-items: center;
    gap: 0.375rem;
    padding: 0.5rem 0.75rem;
    background: #f8fafc;
    border: 1px solid #e2e8f0;
    border-radius: 6px;
    font-size: 0.8125rem;
    font-weight: 500;
    color: #64748b;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .toggle:hover {
    background: #f1f5f9;
    color: #475569;
    border-color: #cbd5e1;
  }

  .sql-panel {
    margin-top: 0.75rem;
    background: #1e293b;
    border-radius: 8px;
    overflow: hidden;
  }

  .sql-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0.625rem 1rem;
    background: #334155;
    border-bottom: 1px solid #475569;
  }

  .sql-title {
    font-size: 0.75rem;
    font-weight: 600;
    color: #94a3b8;
    text-transform: uppercase;
    letter-spacing: 0.025em;
  }

  .copy-btn {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    padding: 0.25rem 0.5rem;
    background: transparent;
    border: 1px solid #475569;
    border-radius: 4px;
    font-size: 0.6875rem;
    font-weight: 500;
    color: #94a3b8;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .copy-btn:hover {
    background: #475569;
    color: #e2e8f0;
  }

  pre {
    margin: 0;
    padding: 1rem;
    overflow-x: auto;
  }

  code {
    font-family: 'SF Mono', 'Monaco', 'Inconsolata', 'Fira Code', monospace;
    font-size: 0.8125rem;
    line-height: 1.6;
    color: #e2e8f0;
    white-space: pre-wrap;
    word-break: break-word;
  }
</style>
