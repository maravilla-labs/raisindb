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
            <span class="progress-filename" title={upload.filename}>{upload.filename}</span>
            {#if upload.status === 'uploading'}
              <span class="progress-percent">{Math.round(upload.progress)}%</span>
            {:else if upload.status === 'error'}
              <span class="progress-error" title={upload.error}>Failed</span>
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
    background: white;
    border: 2px dashed #e2e8f0;
    border-radius: 0.75rem;
    transition: border-color 0.15s, background-color 0.15s;
  }

  .drop-zone.dragging {
    border-color: #3b82f6;
    background: #eff6ff;
  }

  .drag-overlay {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    background: rgba(59, 130, 246, 0.1);
    border-radius: 0.75rem;
    z-index: 10;
    color: #3b82f6;
    pointer-events: none;
  }

  .drag-overlay p {
    margin: 1rem 0 0;
    font-size: 1.125rem;
    font-weight: 500;
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
    background: white;
    color: #475569;
    border: 1px solid #e2e8f0;
    border-radius: 0.5rem;
    font-size: 0.875rem;
    font-weight: 500;
    cursor: pointer;
    box-shadow: 0 2px 4px rgba(0, 0, 0, 0.05);
    transition: background-color 0.15s, border-color 0.15s;
  }

  .upload-btn:hover {
    background: #f8fafc;
    border-color: #3b82f6;
    color: #3b82f6;
  }

  /* Upload progress panel */
  .upload-progress-panel {
    position: fixed;
    bottom: 1.5rem;
    right: 1.5rem;
    width: 320px;
    background: white;
    border-radius: 0.75rem;
    box-shadow: 0 10px 25px rgba(0, 0, 0, 0.15);
    overflow: hidden;
    z-index: 100;
  }

  .progress-header {
    padding: 0.75rem 1rem;
    background: #f8fafc;
    border-bottom: 1px solid #e2e8f0;
    font-size: 0.875rem;
    font-weight: 500;
    color: #475569;
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
    border-bottom: 1px solid #f1f5f9;
  }

  .progress-item:last-child {
    border-bottom: none;
  }

  .progress-icon {
    flex-shrink: 0;
    color: #3b82f6;
  }

  .progress-item.completed .progress-icon {
    color: #10b981;
  }

  .progress-item.error .progress-icon {
    color: #ef4444;
  }

  .spinner {
    width: 16px;
    height: 16px;
    border: 2px solid #e2e8f0;
    border-top-color: #3b82f6;
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .progress-filename {
    flex: 1;
    font-size: 0.875rem;
    color: #1e293b;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .progress-percent {
    font-size: 0.75rem;
    color: #64748b;
    flex-shrink: 0;
  }

  .progress-error {
    font-size: 0.75rem;
    color: #ef4444;
    flex-shrink: 0;
  }
</style>
