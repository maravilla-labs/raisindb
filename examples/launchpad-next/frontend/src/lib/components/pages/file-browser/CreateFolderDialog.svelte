<script lang="ts">
  import { X, FolderPlus } from 'lucide-svelte';

  interface Props {
    onClose: () => void;
    onCreate: (name: string) => Promise<void>;
  }

  let { onClose, onCreate }: Props = $props();

  let folderName = $state('');
  let creating = $state(false);
  let error = $state<string | null>(null);

  async function handleCreate() {
    const name = folderName.trim();
    if (!name) {
      error = 'Please enter a folder name';
      return;
    }

    // Basic validation
    if (name.includes('/') || name.includes('\\')) {
      error = 'Folder name cannot contain slashes';
      return;
    }

    creating = true;
    error = null;

    try {
      await onCreate(name);
    } catch (err) {
      error = err instanceof Error ? err.message : String(err);
      creating = false;
    }
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter' && !creating) {
      handleCreate();
    } else if (e.key === 'Escape') {
      onClose();
    }
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="dialog-overlay" onclick={onClose}>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="dialog" onclick={(e) => e.stopPropagation()}>
    <header class="dialog-header">
      <div class="header-icon">
        <FolderPlus size={20} />
      </div>
      <h2>New Folder</h2>
      <button class="close-btn" onclick={onClose} disabled={creating}>
        <X size={20} />
      </button>
    </header>

    <div class="dialog-body">
      <label for="folder-name">Folder name</label>
      <input
        id="folder-name"
        type="text"
        placeholder="Enter folder name..."
        bind:value={folderName}
        onkeydown={handleKeydown}
        disabled={creating}
        autofocus
      />
      {#if error}
        <p class="error-message">{error}</p>
      {/if}
    </div>

    <footer class="dialog-footer">
      <button class="btn-secondary" onclick={onClose} disabled={creating}>
        Cancel
      </button>
      <button class="btn-primary" onclick={handleCreate} disabled={creating || !folderName.trim()}>
        {#if creating}
          Creating...
        {:else}
          Create Folder
        {/if}
      </button>
    </footer>
  </div>
</div>

<style>
  .dialog-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.7);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1000;
    padding: 1rem;
  }

  .dialog {
    background: var(--color-bg-card);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-lg);
    width: 100%;
    max-width: 400px;
    box-shadow: var(--shadow-lg);
  }

  .dialog-header {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 1.25rem 1.5rem;
    border-bottom: 1px solid var(--color-border);
  }

  .header-icon {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 36px;
    height: 36px;
    background: var(--color-warning-muted);
    color: var(--color-warning);
    border-radius: var(--radius-sm);
    border: 1px solid rgba(245, 158, 11, 0.2);
  }

  .dialog-header h2 {
    flex: 1;
    font-size: 1.125rem;
    font-weight: 600;
    font-family: var(--font-display);
    color: var(--color-text-heading);
    margin: 0;
  }

  .close-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 32px;
    height: 32px;
    background: transparent;
    border: none;
    border-radius: var(--radius-sm);
    color: var(--color-text-muted);
    cursor: pointer;
    transition: background-color 0.15s, color 0.15s;
  }

  .close-btn:hover:not(:disabled) {
    background: var(--color-surface);
    color: var(--color-text);
  }

  .close-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .dialog-body {
    padding: 1.5rem;
  }

  .dialog-body label {
    display: block;
    font-size: 0.875rem;
    font-weight: 500;
    font-family: var(--font-body);
    color: var(--color-text-secondary);
    margin-bottom: 0.5rem;
  }

  .dialog-body input {
    width: 100%;
    padding: 0.75rem 1rem;
    background: var(--color-surface);
    color: var(--color-text);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    font-size: 0.9375rem;
    font-family: var(--font-body);
    transition: border-color 0.15s, box-shadow 0.15s;
  }

  .dialog-body input::placeholder {
    color: var(--color-text-muted);
  }

  .dialog-body input:focus {
    outline: none;
    border-color: var(--color-accent);
    box-shadow: 0 0 0 3px var(--color-accent-glow);
  }

  .dialog-body input:disabled {
    background: var(--color-bg-elevated);
    color: var(--color-text-muted);
  }

  .error-message {
    margin: 0.5rem 0 0;
    font-size: 0.875rem;
    color: var(--color-error);
  }

  .dialog-footer {
    display: flex;
    justify-content: flex-end;
    gap: 0.75rem;
    padding: 1rem 1.5rem;
    background: var(--color-bg-elevated);
    border-top: 1px solid var(--color-border);
    border-radius: 0 0 var(--radius-lg) var(--radius-lg);
  }

  .btn-secondary,
  .btn-primary {
    padding: 0.625rem 1.25rem;
    border-radius: var(--radius-sm);
    font-size: 0.875rem;
    font-weight: 500;
    font-family: var(--font-body);
    cursor: pointer;
    transition: background-color 0.15s, opacity 0.15s;
  }

  .btn-secondary {
    background: var(--color-surface);
    color: var(--color-text);
    border: 1px solid var(--color-border);
  }

  .btn-secondary:hover:not(:disabled) {
    background: var(--color-bg-card);
  }

  .btn-primary {
    background: var(--color-accent);
    color: var(--color-bg);
    border: none;
  }

  .btn-primary:hover:not(:disabled) {
    background: var(--color-accent-hover);
  }

  .btn-secondary:disabled,
  .btn-primary:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
</style>
