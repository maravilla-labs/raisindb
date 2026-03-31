<script lang="ts">
  import { onMount } from 'svelte';
  import { Upload, CheckCircle, AlertCircle, X } from 'lucide-svelte';
  import type { Snippet } from 'svelte';
  import type { UploadProgressItem } from '../FileBrowserPage.svelte';

  interface Props {
    onFilesDropped: (files: File[]) => void;
    uploads: UploadProgressItem[];
    children: Snippet;
  }

  let { onFilesDropped, uploads, children }: Props = $props();

  let isDragging = $state(false);
  let fileInput: HTMLInputElement;
  let dropZoneElement: HTMLDivElement;

  // Check if this is an external file drag (from filesystem)
  function isFileDrag(e: DragEvent): boolean {
    return e.dataTransfer?.types.includes('Files') ?? false;
  }

  function handleDragOver(e: DragEvent) {
    e.preventDefault();
    // Only show upload overlay for external file drags, not internal reordering
    if (isFileDrag(e)) {
      isDragging = true;
    }
  }

  function handleDragLeave(e: DragEvent) {
    e.preventDefault();
    // Only set to false if we're leaving the dropzone entirely
    const relatedTarget = e.relatedTarget as HTMLElement;
    if (!relatedTarget?.closest('.drop-zone')) {
      isDragging = false;
    }
  }

  function handleDrop(e: DragEvent) {
    e.preventDefault();
    isDragging = false;

    // Only handle external file drops
    if (!isFileDrag(e)) return;

    const files = Array.from(e.dataTransfer?.files || []);
    if (files.length > 0) {
      onFilesDropped(files);
    }
  }

  // Reset dragging state on any drop inside (capture phase)
  // This ensures we reset even if a child stops propagation
  function handleDropCapture() {
    isDragging = false;
  }

  // Attach capture phase listener for drop events
  onMount(() => {
    dropZoneElement?.addEventListener('drop', handleDropCapture, true);
    return () => {
      dropZoneElement?.removeEventListener('drop', handleDropCapture, true);
    };
  });

  function handleFileSelect(e: Event) {
    const input = e.target as HTMLInputElement;
    const files = Array.from(input.files || []);
    if (files.length > 0) {
      onFilesDropped(files);
    }
    // Reset input so same file can be selected again
    input.value = '';
  }

  function openFilePicker() {
    fileInput?.click();
  }

  // Check if there are active uploads
  const hasActiveUploads = $derived(uploads.some(u => u.status === 'uploading'));
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  bind:this={dropZoneElement}
  class="drop-zone"
  class:dragging={isDragging}
  ondragover={handleDragOver}
  ondragleave={handleDragLeave}
  ondrop={handleDrop}
>
  {#if isDragging}
    <div class="drag-overlay">
      <Upload size={48} />
      <p>Drop files here to upload</p>
    </div>
  {/if}

  <div class="content-area">
    {@render children()}
  </div>

  <!-- Upload button -->
  <div class="upload-actions">
    <input
      bind:this={fileInput}
      type="file"
      multiple
      onchange={handleFileSelect}
      class="file-input"
    />
    <button class="upload-btn" onclick={openFilePicker}>
      <Upload size={18} />
      <span>Upload Files</span>
    </button>
  </div>

  <!-- Upload progress panel -->
  {#if uploads.length > 0}
    <div class="upload-progress-panel">
      <div class="progress-header">
        <span>Uploads ({uploads.filter(u => u.status === 'completed').length}/{uploads.length})</span>
      </div>
      <ul class="progress-list">
        {#each uploads as upload (upload.id)}
          <li class="progress-item" class:completed={upload.status === 'completed'} class:error={upload.status === 'error'}>
            <div class="progress-icon">
              {#if upload.status === 'completed'}
                <CheckCircle size={16} />
              {:else if upload.status === 'error'}
                <AlertCircle size={16} />
              {:else}
                <div class="spinner"></div>
              {/if}
            </div>
            <div class="progress-details">
              <span class="progress-filename" title={upload.filename}>{upload.filename}</span>
              {#if upload.status === 'error' && upload.error}
                <span class="progress-error-detail">{upload.error}</span>
              {/if}
            </div>
            {#if upload.status === 'uploading'}
              <span class="progress-percent">{Math.round(upload.progress)}%</span>
            {:else if upload.status === 'error'}
              <span class="progress-error">Failed</span>
            {/if}
          </li>
        {/each}
      </ul>
    </div>
  {/if}
</div>

<style>
  .drop-zone {
    position: relative;
    min-height: 400px;
    background: var(--color-bg-card);
    border: 2px dashed var(--color-border);
    border-radius: var(--radius-md);
    transition: border-color 0.15s, background-color 0.15s;
  }

  .drop-zone.dragging {
    border-color: var(--color-accent);
    background: var(--color-accent-muted);
  }

  .drag-overlay {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    background: rgba(212, 175, 55, 0.08);
    border-radius: var(--radius-md);
    z-index: 10;
    color: var(--color-accent);
    pointer-events: none;
  }

  .drag-overlay p {
    margin: 1rem 0 0;
    font-size: 1.125rem;
    font-weight: 500;
    font-family: var(--font-body);
  }

  .content-area {
    min-height: 300px;
  }

  .upload-actions {
    position: absolute;
    bottom: 1rem;
    right: 1rem;
  }

  .file-input {
    display: none;
  }

  .upload-btn {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.625rem 1rem;
    background: var(--color-surface);
    color: var(--color-text-secondary);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    font-size: 0.875rem;
    font-weight: 500;
    font-family: var(--font-body);
    cursor: pointer;
    transition: background-color 0.15s, border-color 0.15s, color 0.15s;
  }

  .upload-btn:hover {
    background: var(--color-bg-card);
    border-color: var(--color-accent);
    color: var(--color-accent);
  }

  /* Upload progress panel */
  .upload-progress-panel {
    position: fixed;
    bottom: 1.5rem;
    right: 1.5rem;
    width: 320px;
    background: var(--color-bg-card);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-md);
    box-shadow: var(--shadow-lg);
    overflow: hidden;
    z-index: 100;
  }

  .progress-header {
    padding: 0.75rem 1rem;
    background: var(--color-bg-elevated);
    border-bottom: 1px solid var(--color-border);
    font-size: 0.875rem;
    font-weight: 500;
    font-family: var(--font-body);
    color: var(--color-text-secondary);
  }

  .progress-list {
    max-height: 240px;
    overflow-y: auto;
    list-style: none;
    margin: 0;
    padding: 0;
  }

  .progress-item {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0.75rem 1rem;
    border-bottom: 1px solid var(--color-border);
  }

  .progress-item:last-child {
    border-bottom: none;
  }

  .progress-icon {
    flex-shrink: 0;
    color: var(--color-accent);
  }

  .progress-item.completed .progress-icon {
    color: var(--color-success);
  }

  .progress-item.error .progress-icon {
    color: var(--color-error);
  }

  .spinner {
    width: 16px;
    height: 16px;
    border: 2px solid var(--color-border);
    border-top-color: var(--color-accent);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .progress-details {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 0.125rem;
  }

  .progress-filename {
    font-size: 0.875rem;
    color: var(--color-text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .progress-error-detail {
    font-size: 0.75rem;
    color: var(--color-text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .progress-percent {
    font-size: 0.75rem;
    color: var(--color-text-secondary);
    flex-shrink: 0;
  }

  .progress-error {
    font-size: 0.75rem;
    color: var(--color-error);
    flex-shrink: 0;
  }
</style>
