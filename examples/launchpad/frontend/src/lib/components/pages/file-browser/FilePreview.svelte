<script lang="ts">
  import { X, Download, File, ExternalLink, Loader2, Pencil, Plus } from 'lucide-svelte';
  import type { FileItem } from '../FileBrowserPage.svelte';
  import { signAssetUrl, updateAsset } from '$lib/raisin';

  interface Props {
    item: FileItem;
    onClose: () => void;
    onUpdate?: () => void;
  }

  let { item, onClose, onUpdate }: Props = $props();

  // Signed URL state
  let displayUrl = $state<string | null>(null);
  let downloadUrl = $state<string | null>(null);
  let loadingUrls = $state(true);
  let urlError = $state<string | null>(null);

  // Edit mode state
  let isEditing = $state(false);
  let isSaving = $state(false);
  let editDescription = $state(item.properties.description || '');
  let editAltText = $state(item.properties.alt_text || '');
  let editKeywords = $state<string[]>(item.properties.keywords || []);
  let newKeyword = $state('');

  // Check if there's any metadata to display
  const hasMetadata = $derived(
    item.properties.description ||
    item.properties.alt_text ||
    item.properties.caption ||
    (item.properties.keywords && item.properties.keywords.length > 0)
  );

  // Get MIME type
  const mimeType = $derived(item.properties.file_type || item.properties.file?.mime_type || '');

  // Determine preview type
  const previewType = $derived(() => {
    if (mimeType.startsWith('image/')) return 'image';
    if (mimeType === 'application/pdf') return 'pdf';
    if (mimeType === 'text/html') return 'html';
    if (mimeType.startsWith('video/')) return 'video';
    if (mimeType.startsWith('audio/')) return 'audio';
    return 'none';
  });

  // Get display name
  const displayName = $derived(item.properties.title || item.properties.file?.name || item.name);

  // Format file size
  function formatSize(bytes: number | undefined): string {
    if (!bytes) return '';
    if (bytes < 1024) return bytes + ' B';
    if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + ' KB';
    if (bytes < 1024 * 1024 * 1024) return (bytes / (1024 * 1024)).toFixed(1) + ' MB';
    return (bytes / (1024 * 1024 * 1024)).toFixed(1) + ' GB';
  }

  // Download file using signed URL
  function downloadFile() {
    if (downloadUrl) {
      window.open(downloadUrl, '_blank');
    }
  }

  // Handle keyboard events
  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      if (isEditing) {
        cancelEdit();
      } else {
        onClose();
      }
    }
  }

  // Start editing
  function startEdit() {
    editDescription = item.properties.description || '';
    editAltText = item.properties.alt_text || '';
    editKeywords = [...(item.properties.keywords || [])];
    newKeyword = '';
    isEditing = true;
  }

  // Cancel editing
  function cancelEdit() {
    isEditing = false;
    newKeyword = '';
  }

  // Add keyword
  function addKeyword() {
    const keyword = newKeyword.trim();
    if (keyword && !editKeywords.includes(keyword)) {
      editKeywords = [...editKeywords, keyword];
    }
    newKeyword = '';
  }

  // Remove keyword
  function removeKeyword(keyword: string) {
    editKeywords = editKeywords.filter(k => k !== keyword);
  }

  // Handle keyword input keydown
  function handleKeywordKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter') {
      e.preventDefault();
      addKeyword();
    }
  }

  // Save metadata
  async function saveMetadata() {
    isSaving = true;
    try {
      await updateAsset(item.path, {
        description: editDescription || null,
        alt_text: editAltText || null,
        keywords: editKeywords.length > 0 ? editKeywords : null
      });
      // Update local item state
      item.properties.description = editDescription || undefined;
      item.properties.alt_text = editAltText || undefined;
      item.properties.keywords = editKeywords.length > 0 ? editKeywords : undefined;
      isEditing = false;
      onUpdate?.();
    } catch (err) {
      console.error('[file-preview] Failed to save metadata:', err);
    } finally {
      isSaving = false;
    }
  }

  // Fetch signed URLs when item changes
  $effect(() => {
    const nodePath = item.path;
    loadingUrls = true;
    urlError = null;

    Promise.all([
      signAssetUrl(nodePath, 'display'),
      signAssetUrl(nodePath, 'download')
    ])
      .then(([display, download]) => {
        displayUrl = display.url;
        downloadUrl = download.url;
        loadingUrls = false;
      })
      .catch((err) => {
        console.error('[file-preview] Failed to get signed URLs:', err);
        urlError = err instanceof Error ? err.message : String(err);
        loadingUrls = false;
      });
  });

  // Lock body scroll when modal is open
  $effect(() => {
    const originalOverflow = document.body.style.overflow;
    document.body.style.overflow = 'hidden';

    return () => {
      document.body.style.overflow = originalOverflow;
    };
  });
</script>

<svelte:window onkeydown={handleKeydown} />

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="preview-overlay" onclick={onClose}>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="preview-modal" onclick={(e) => e.stopPropagation()}>
    <header class="preview-header">
      <div class="preview-title">
        <h2>{displayName}</h2>
        {#if item.properties.file?.size}
          <span class="file-size">{formatSize(item.properties.file.size)}</span>
        {/if}
      </div>
      <div class="preview-actions">
        <button
          class="action-btn"
          onclick={downloadFile}
          title="Download"
          disabled={loadingUrls || !downloadUrl}
        >
          <Download size={18} />
        </button>
        <button class="action-btn" onclick={onClose} title="Close">
          <X size={18} />
        </button>
      </div>
    </header>

    <div class="preview-content">
      {#if loadingUrls}
        <div class="loading-state">
          <Loader2 size={32} class="spinning" />
          <p>Loading preview...</p>
        </div>
      {:else if urlError}
        <div class="preview-fallback">
          <File size={64} />
          <p class="fallback-filename">{displayName}</p>
          <p class="fallback-type error">Failed to load: {urlError}</p>
        </div>
      {:else if previewType() === 'image' && displayUrl}
        <img
          src={displayUrl}
          alt={item.properties.alt_text || displayName}
          title={item.properties.alt_text || displayName}
          class="preview-image"
        />
      {:else if previewType() === 'pdf' && displayUrl}
        <iframe src={displayUrl} title={displayName} class="preview-iframe"></iframe>
      {:else if previewType() === 'html' && displayUrl}
        <iframe src={displayUrl} title={displayName} class="preview-iframe" sandbox="allow-scripts allow-same-origin"></iframe>
      {:else if previewType() === 'video' && displayUrl}
        <video src={displayUrl} controls class="preview-video">
          <track kind="captions" />
        </video>
      {:else if previewType() === 'audio' && displayUrl}
        <div class="preview-audio-container">
          <File size={64} />
          <p class="audio-filename">{displayName}</p>
          <audio src={displayUrl} controls class="preview-audio"></audio>
        </div>
      {:else}
        <div class="preview-fallback">
          <File size={64} />
          <p class="fallback-filename">{displayName}</p>
          <p class="fallback-type">{mimeType || 'Unknown type'}</p>
          <button class="download-btn" onclick={downloadFile}>
            <Download size={18} />
            <span>Download File</span>
          </button>
          {#if displayUrl}
            <a href={displayUrl} target="_blank" rel="noopener noreferrer" class="open-link">
              <ExternalLink size={14} />
              <span>Open in new tab</span>
            </a>
          {/if}
        </div>
      {/if}
    </div>

    <!-- Metadata Panel -->
    {#if item.node_type === 'raisin:Asset'}
      <div class="metadata-panel">
        {#if isEditing}
          <!-- Edit Mode -->
          <div class="metadata-edit">
            <div class="field-group">
              <label for="edit-description">Description</label>
              <textarea
                id="edit-description"
                bind:value={editDescription}
                placeholder="Enter a description..."
                rows="2"
              ></textarea>
            </div>

            <div class="field-group">
              <label for="edit-alt-text">Alt Text</label>
              <input
                type="text"
                id="edit-alt-text"
                bind:value={editAltText}
                placeholder="Enter alt text for accessibility..."
              />
            </div>

            <div class="field-group">
              <label>Keywords</label>
              <div class="keywords-edit">
                {#each editKeywords as keyword}
                  <span class="keyword-tag editable">
                    {keyword}
                    <button
                      type="button"
                      class="keyword-remove"
                      onclick={() => removeKeyword(keyword)}
                      title="Remove keyword"
                    >
                      <X size={12} />
                    </button>
                  </span>
                {/each}
                <div class="keyword-input-wrapper">
                  <input
                    type="text"
                    bind:value={newKeyword}
                    placeholder="Add keyword..."
                    onkeydown={handleKeywordKeydown}
                  />
                  <button
                    type="button"
                    class="keyword-add"
                    onclick={addKeyword}
                    disabled={!newKeyword.trim()}
                    title="Add keyword"
                  >
                    <Plus size={14} />
                  </button>
                </div>
              </div>
            </div>

            <div class="edit-actions">
              <button class="btn-secondary" onclick={cancelEdit} disabled={isSaving}>
                Cancel
              </button>
              <button class="btn-primary" onclick={saveMetadata} disabled={isSaving}>
                {#if isSaving}
                  <Loader2 size={14} class="spinning" />
                  Saving...
                {:else}
                  Save
                {/if}
              </button>
            </div>
          </div>
        {:else}
          <!-- View Mode -->
          <div class="metadata-view">
            {#if hasMetadata}
              {#if item.properties.description}
                <div class="metadata-row">
                  <span class="metadata-label">Description</span>
                  <span class="metadata-value">{item.properties.description}</span>
                </div>
              {/if}

              {#if item.properties.alt_text || item.properties.caption}
                <div class="metadata-row">
                  <span class="metadata-label">Alt Text</span>
                  <span class="metadata-value">{item.properties.alt_text || item.properties.caption}</span>
                </div>
              {/if}

              {#if item.properties.keywords && item.properties.keywords.length > 0}
                <div class="metadata-row">
                  <span class="metadata-label">Keywords</span>
                  <div class="keywords-display">
                    {#each item.properties.keywords as keyword}
                      <span class="keyword-tag">{keyword}</span>
                    {/each}
                  </div>
                </div>
              {/if}
            {:else}
              <p class="no-metadata">No metadata available</p>
            {/if}

            <button class="edit-btn" onclick={startEdit} title="Edit metadata">
              <Pencil size={14} />
              <span>Edit</span>
            </button>
          </div>
        {/if}
      </div>
    {/if}
  </div>
</div>

<style>
  .preview-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.8);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1000;
    padding: 2rem;
  }

  .preview-modal {
    background: white;
    border-radius: 1rem;
    width: 100%;
    max-width: 900px;
    max-height: 90vh;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    box-shadow: 0 20px 40px rgba(0, 0, 0, 0.3);
  }

  .preview-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 1rem 1.5rem;
    border-bottom: 1px solid #e2e8f0;
    background: #f8fafc;
  }

  .preview-title {
    display: flex;
    align-items: baseline;
    gap: 0.75rem;
    min-width: 0;
  }

  .preview-title h2 {
    font-size: 1.125rem;
    font-weight: 600;
    color: #1e293b;
    margin: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .file-size {
    font-size: 0.875rem;
    color: #64748b;
    flex-shrink: 0;
  }

  .preview-actions {
    display: flex;
    gap: 0.5rem;
    flex-shrink: 0;
  }

  .action-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 36px;
    height: 36px;
    background: white;
    border: 1px solid #e2e8f0;
    border-radius: 0.5rem;
    color: #64748b;
    cursor: pointer;
    transition: color 0.15s, background-color 0.15s;
  }

  .action-btn:hover:not(:disabled) {
    color: #3b82f6;
    background: #f1f5f9;
  }

  .action-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .preview-content {
    flex: 1;
    overflow: auto;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 1.5rem;
    background: #f1f5f9;
  }

  .preview-image {
    max-width: 100%;
    max-height: 70vh;
    object-fit: contain;
    border-radius: 0.5rem;
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.15);
  }

  .preview-iframe {
    width: 100%;
    height: 70vh;
    border: none;
    border-radius: 0.5rem;
    background: white;
  }

  .preview-video {
    max-width: 100%;
    max-height: 70vh;
    border-radius: 0.5rem;
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.15);
  }

  .preview-audio-container {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 1rem;
    padding: 2rem;
    color: #64748b;
  }

  .audio-filename {
    font-size: 1rem;
    font-weight: 500;
    color: #1e293b;
    margin: 0;
  }

  .preview-audio {
    width: 100%;
    max-width: 400px;
  }

  .preview-fallback {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 1rem;
    padding: 3rem;
    color: #64748b;
    text-align: center;
  }

  .fallback-filename {
    font-size: 1.125rem;
    font-weight: 500;
    color: #1e293b;
    margin: 0;
  }

  .fallback-type {
    font-size: 0.875rem;
    color: #94a3b8;
    margin: 0;
  }

  .download-btn {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.75rem 1.5rem;
    background: #3b82f6;
    color: white;
    border: none;
    border-radius: 0.5rem;
    font-size: 0.875rem;
    font-weight: 500;
    cursor: pointer;
    transition: background-color 0.15s;
    margin-top: 0.5rem;
  }

  .download-btn:hover {
    background: #2563eb;
  }

  .open-link {
    display: flex;
    align-items: center;
    gap: 0.375rem;
    font-size: 0.875rem;
    color: #3b82f6;
    text-decoration: none;
    margin-top: 0.5rem;
  }

  .open-link:hover {
    text-decoration: underline;
  }

  .loading-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 1rem;
    padding: 3rem;
    color: #64748b;
  }

  .loading-state p {
    margin: 0;
    font-size: 0.875rem;
  }

  .loading-state :global(.spinning) {
    animation: spin 1s linear infinite;
  }

  @keyframes spin {
    from { transform: rotate(0deg); }
    to { transform: rotate(360deg); }
  }

  .fallback-type.error {
    color: #ef4444;
  }

  /* Metadata Panel */
  .metadata-panel {
    border-top: 1px solid #e2e8f0;
    padding: 1rem 1.5rem;
    background: white;
    max-height: 40vh;
    overflow-y: auto;
  }

  .metadata-view {
    position: relative;
  }

  .metadata-row {
    display: flex;
    gap: 1rem;
    margin-bottom: 0.75rem;
  }

  .metadata-row:last-of-type {
    margin-bottom: 0;
  }

  .metadata-label {
    font-size: 0.75rem;
    font-weight: 600;
    color: #64748b;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    min-width: 80px;
    flex-shrink: 0;
  }

  .metadata-value {
    font-size: 0.875rem;
    color: #1e293b;
    line-height: 1.5;
  }

  .keywords-display {
    display: flex;
    flex-wrap: wrap;
    gap: 0.375rem;
  }

  .keyword-tag {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    padding: 0.25rem 0.5rem;
    background: #e0f2fe;
    color: #0369a1;
    border-radius: 0.25rem;
    font-size: 0.75rem;
    font-weight: 500;
  }

  .keyword-tag.editable {
    background: #dbeafe;
    padding-right: 0.25rem;
  }

  .keyword-remove {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0.125rem;
    background: transparent;
    border: none;
    color: #64748b;
    cursor: pointer;
    border-radius: 0.125rem;
    transition: color 0.15s, background-color 0.15s;
  }

  .keyword-remove:hover {
    color: #dc2626;
    background: #fee2e2;
  }

  .no-metadata {
    font-size: 0.875rem;
    color: #94a3b8;
    margin: 0;
  }

  .edit-btn {
    position: absolute;
    top: 0;
    right: 0;
    display: flex;
    align-items: center;
    gap: 0.375rem;
    padding: 0.375rem 0.75rem;
    background: transparent;
    border: 1px solid #e2e8f0;
    border-radius: 0.375rem;
    color: #64748b;
    font-size: 0.75rem;
    cursor: pointer;
    transition: color 0.15s, border-color 0.15s, background-color 0.15s;
  }

  .edit-btn:hover {
    color: #3b82f6;
    border-color: #3b82f6;
    background: #eff6ff;
  }

  /* Edit Mode */
  .metadata-edit {
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }

  .field-group {
    display: flex;
    flex-direction: column;
    gap: 0.375rem;
  }

  .field-group label {
    font-size: 0.75rem;
    font-weight: 600;
    color: #64748b;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .field-group input,
  .field-group textarea {
    padding: 0.5rem 0.75rem;
    border: 1px solid #e2e8f0;
    border-radius: 0.375rem;
    font-size: 0.875rem;
    color: #1e293b;
    transition: border-color 0.15s, box-shadow 0.15s;
  }

  .field-group input:focus,
  .field-group textarea:focus {
    outline: none;
    border-color: #3b82f6;
    box-shadow: 0 0 0 3px rgba(59, 130, 246, 0.1);
  }

  .field-group textarea {
    resize: vertical;
    min-height: 60px;
  }

  .keywords-edit {
    display: flex;
    flex-wrap: wrap;
    gap: 0.375rem;
    align-items: center;
  }

  .keyword-input-wrapper {
    display: flex;
    gap: 0.25rem;
    align-items: center;
  }

  .keyword-input-wrapper input {
    padding: 0.25rem 0.5rem;
    font-size: 0.75rem;
    width: 120px;
  }

  .keyword-add {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    padding: 0;
    background: #3b82f6;
    border: none;
    border-radius: 0.25rem;
    color: white;
    cursor: pointer;
    transition: background-color 0.15s;
  }

  .keyword-add:hover:not(:disabled) {
    background: #2563eb;
  }

  .keyword-add:disabled {
    background: #94a3b8;
    cursor: not-allowed;
  }

  .edit-actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.5rem;
    padding-top: 0.5rem;
    border-top: 1px solid #e2e8f0;
  }

  .btn-secondary,
  .btn-primary {
    display: flex;
    align-items: center;
    gap: 0.375rem;
    padding: 0.5rem 1rem;
    border-radius: 0.375rem;
    font-size: 0.875rem;
    font-weight: 500;
    cursor: pointer;
    transition: background-color 0.15s, border-color 0.15s;
  }

  .btn-secondary {
    background: white;
    border: 1px solid #e2e8f0;
    color: #64748b;
  }

  .btn-secondary:hover:not(:disabled) {
    background: #f8fafc;
    border-color: #cbd5e1;
  }

  .btn-primary {
    background: #3b82f6;
    border: 1px solid #3b82f6;
    color: white;
  }

  .btn-primary:hover:not(:disabled) {
    background: #2563eb;
    border-color: #2563eb;
  }

  .btn-secondary:disabled,
  .btn-primary:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  .btn-primary :global(.spinning) {
    animation: spin 1s linear infinite;
  }
</style>
