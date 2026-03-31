<script lang="ts">
  import {
    Folder,
    File,
    FileText,
    FileImage,
    FileVideo,
    FileAudio,
    FileCode,
    FileArchive,
    FileSpreadsheet
  } from 'lucide-svelte';
  import type { FileItem } from '../FileBrowserPage.svelte';
  import { signAssetUrl } from '$lib/raisin';

  interface Props {
    items: FileItem[];
    onItemClick: (item: FileItem) => void;
    onDropToFolder?: (files: File[], folderPath: string) => void;
    onReorder?: (source: FileItem, target: FileItem, position: 'above' | 'below') => void;
    onMoveToFolder?: (source: FileItem, targetFolderPath: string) => void;
  }

  let { items, onItemClick, onDropToFolder, onReorder, onMoveToFolder }: Props = $props();

  // Track which folder is being dragged over (for file uploads)
  let dragOverFolderId = $state<string | null>(null);

  // Track reordering state (gap insertion approach)
  let dragSourceItem = $state<FileItem | null>(null);
  let dragSourceIndex = $state<number | null>(null);
  let dropIndex = $state<number | null>(null);
  // Also track target by ID to handle items array changes during drag
  let dropTargetId = $state<string | null>(null);
  let dropPosition = $state<'above' | 'below' | null>(null);

  // Track "move into folder" state (when dragging over folder center)
  let moveIntoFolderId = $state<string | null>(null);

  // Thumbnail URLs state - maps item.id to signed URL
  let thumbnailUrls = $state<Map<string, string>>(new Map());

  // Fetch thumbnail URLs for items that have a thumbnail property
  $effect(() => {
    const itemsWithThumbnails = items.filter(
      item => item.node_type === 'raisin:Asset' && item.properties.thumbnail
    );

    // Fetch thumbnail URLs for items we don't have yet
    for (const item of itemsWithThumbnails) {
      if (!thumbnailUrls.has(item.id)) {
        signAssetUrl(item.path, 'display', { propertyPath: 'thumbnail' })
          .then(({ url }) => {
            thumbnailUrls = new Map(thumbnailUrls).set(item.id, url);
          })
          .catch((err) => {
            console.error(`[file-grid] Failed to get thumbnail URL for ${item.path}:`, err);
          });
      }
    }

    // Clean up URLs for items that are no longer in the list
    const currentIds = new Set(items.map(i => i.id));
    const urlsToRemove = [...thumbnailUrls.keys()].filter(id => !currentIds.has(id));
    if (urlsToRemove.length > 0) {
      const newMap = new Map(thumbnailUrls);
      for (const id of urlsToRemove) {
        newMap.delete(id);
      }
      thumbnailUrls = newMap;
    }
  });

  // Clear drag state when items change (prevents stale references after navigation/move/refresh)
  $effect(() => {
    // Subscribe to items changes
    const _ = items;
    // Reset drag state to prevent stale references
    if (dragSourceItem) {
      // Check if the dragged item is still in the current items array
      const stillExists = items.some(i => i.id === dragSourceItem?.id);
      if (!stillExists) {
        handleDragEnd();
      }
    }
  });

  // Check if this is an external file drag (from filesystem)
  function isFileDrag(e: DragEvent): boolean {
    return e.dataTransfer?.types.includes('Files') ?? false;
  }

  // Handle drag start for reordering
  function handleDragStart(e: DragEvent, item: FileItem, index: number) {
    dragSourceItem = item;
    dragSourceIndex = index;
    e.dataTransfer!.effectAllowed = 'move';
    e.dataTransfer!.setData('text/plain', item.id);
  }

  // Handle drag end
  function handleDragEnd() {
    dragSourceItem = null;
    dragSourceIndex = null;
    dropIndex = null;
    dropTargetId = null;
    dropPosition = null;
    dragOverFolderId = null;
    moveIntoFolderId = null;
  }

  // Handle drag over
  function handleDragOver(e: DragEvent, item: FileItem, index: number) {
    e.preventDefault();
    e.stopPropagation();

    // External file drag onto folder
    if (isFileDrag(e)) {
      if (item.node_type === 'raisin:Folder') {
        dragOverFolderId = item.id;
      }
      dropIndex = null;
      dropTargetId = null;
      dropPosition = null;
      moveIntoFolderId = null;
      return;
    }

    // Internal reordering - need a drag source
    if (!dragSourceItem || dragSourceItem.id === item.id) {
      dropIndex = null;
      dropTargetId = null;
      dropPosition = null;
      moveIntoFolderId = null;
      return;
    }

    // Calculate relative position in item (0 to 1)
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const mouseX = e.clientX - rect.left;
    const relativeX = mouseX / rect.width;

    // For folders: center 40% (0.3 to 0.7) is "move into", edges are reorder
    if (item.node_type === 'raisin:Folder') {
      if (relativeX > 0.3 && relativeX < 0.7) {
        // Center zone - move into folder
        moveIntoFolderId = item.id;
        dropIndex = null;
        dropTargetId = null;
        dropPosition = null;
        return;
      }
    }

    // Edge zones - reorder (existing logic)
    moveIntoFolderId = null;
    const isLeftHalf = relativeX < 0.5;

    // Insert before this item if on left half, after if on right half
    let newDropIndex = isLeftHalf ? index : index + 1;

    // Don't show gap at source position or right after it (no-op positions)
    if (dragSourceIndex !== null) {
      if (newDropIndex === dragSourceIndex || newDropIndex === dragSourceIndex + 1) {
        dropIndex = null;
        dropTargetId = null;
        dropPosition = null;
        return;
      }
    }

    dropIndex = newDropIndex;
    // Track target by ID for resilience against items array changes
    dropTargetId = item.id;
    dropPosition = isLeftHalf ? 'above' : 'below';
  }

  // Handle drag leave
  function handleDragLeave(e: DragEvent, item: FileItem) {
    e.preventDefault();
    e.stopPropagation();

    if (isFileDrag(e) && item.node_type === 'raisin:Folder') {
      dragOverFolderId = null;
    }

    // Reset move-into state when leaving
    if (moveIntoFolderId === item.id) {
      moveIntoFolderId = null;
    }
  }

  // Handle drop on item
  function handleDropOnItem(e: DragEvent, item: FileItem) {
    e.preventDefault();
    e.stopPropagation();

    // External file drop onto folder
    if (isFileDrag(e)) {
      dragOverFolderId = null;
      if (item.node_type === 'raisin:Folder') {
        const files = Array.from(e.dataTransfer?.files || []);
        if (files.length > 0 && onDropToFolder) {
          onDropToFolder(files, item.path);
        }
      }
      return;
    }

    // Internal move into folder (center zone drop)
    if (moveIntoFolderId === item.id && item.node_type === 'raisin:Folder' && dragSourceItem) {
      if (onMoveToFolder) {
        onMoveToFolder(dragSourceItem, item.path);
      }
      handleDragEnd();
      return;
    }

    // Internal reordering handled by handleDropOnGrid
  }

  // Handle drop on grid (for gap drops)
  function handleDropOnGrid(e: DragEvent) {
    // Don't handle if it was already handled by an item
    if (e.defaultPrevented) return;

    e.preventDefault();

    // Use ID-based lookup for target (resilient to items array changes)
    if (!dragSourceItem || !dropTargetId || !dropPosition || !onReorder) {
      handleDragEnd();
      return;
    }

    // Find source item in current items (validates it's still there)
    const sourceItem = items.find(i => i.id === dragSourceItem!.id);
    if (!sourceItem) {
      console.warn('[file-grid] Source item no longer in current folder, cancelling reorder');
      handleDragEnd();
      return;
    }

    // Find target item by ID (not by index - indices can become stale)
    const targetItem = items.find(i => i.id === dropTargetId);
    if (!targetItem) {
      console.warn('[file-grid] Target item no longer in current folder, cancelling reorder');
      handleDragEnd();
      return;
    }

    // Skip if same item
    if (targetItem.id === sourceItem.id) {
      handleDragEnd();
      return;
    }

    // Use the fresh source item (with current path) for the reorder
    onReorder(sourceItem, targetItem, dropPosition);

    handleDragEnd();
  }

  // Get icon based on MIME type
  function getFileIcon(item: FileItem) {
    if (item.node_type === 'raisin:Folder') {
      return Folder;
    }

    const mimeType = item.properties.file_type || item.properties.file?.mime_type || '';

    if (mimeType.startsWith('image/')) return FileImage;
    if (mimeType.startsWith('video/')) return FileVideo;
    if (mimeType.startsWith('audio/')) return FileAudio;
    if (mimeType.startsWith('text/')) return FileText;
    if (mimeType === 'application/pdf') return FileText;
    if (mimeType.includes('zip') || mimeType.includes('archive') || mimeType.includes('tar') || mimeType.includes('rar')) return FileArchive;
    if (mimeType.includes('spreadsheet') || mimeType.includes('excel') || mimeType === 'text/csv') return FileSpreadsheet;
    if (mimeType.includes('javascript') || mimeType.includes('json') || mimeType.includes('xml') || mimeType.includes('html')) return FileCode;

    return File;
  }

  // Format file size
  function formatSize(bytes: number | undefined): string {
    if (!bytes) return '';
    if (bytes < 1024) return bytes + ' B';
    if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + ' KB';
    if (bytes < 1024 * 1024 * 1024) return (bytes / (1024 * 1024)).toFixed(1) + ' MB';
    return (bytes / (1024 * 1024 * 1024)).toFixed(1) + ' GB';
  }

  // Get display name
  function getDisplayName(item: FileItem): string {
    return item.properties.title || item.properties.file?.name || item.name;
  }

  // Get accessible label (alt_text for images, or display name)
  function getAccessibleLabel(item: FileItem): string {
    return item.properties.alt_text || getDisplayName(item);
  }

  // Get file size
  function getFileSize(item: FileItem): number | undefined {
    return item.properties.file_size || item.properties.file?.size;
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="file-grid" ondrop={handleDropOnGrid} ondragover={(e) => e.preventDefault()}>
  {#each items as item, index (item.id)}
    {#if dropIndex === index && dragSourceItem?.id !== item.id}
      <div class="drop-gap"></div>
    {/if}
    <button
      class="file-item"
      class:is-folder={item.node_type === 'raisin:Folder'}
      class:drag-over={dragOverFolderId === item.id}
      class:move-into={moveIntoFolderId === item.id}
      class:dragging={dragSourceItem?.id === item.id}
      draggable="true"
      title={getAccessibleLabel(item)}
      aria-label={getAccessibleLabel(item)}
      onclick={() => onItemClick(item)}
      ondragstart={(e) => handleDragStart(e, item, index)}
      ondragend={handleDragEnd}
      ondragover={(e) => handleDragOver(e, item, index)}
      ondragleave={(e) => handleDragLeave(e, item)}
      ondrop={(e) => handleDropOnItem(e, item)}
    >
      <div class="file-icon" class:folder={item.node_type === 'raisin:Folder'} class:has-thumbnail={thumbnailUrls.has(item.id)}>
        {#if thumbnailUrls.has(item.id)}
          <img src={thumbnailUrls.get(item.id)} alt={getAccessibleLabel(item)} />
          <span class="file-type-badge">
            <svelte:component this={getFileIcon(item)} size={14} />
          </span>
        {:else}
          <svelte:component this={getFileIcon(item)} size={32} />
        {/if}
      </div>
      <div class="file-info">
        <span class="file-name" title={getDisplayName(item)}>
          {getDisplayName(item)}
        </span>
        {#if item.node_type !== 'raisin:Folder'}
          <span class="file-size">{formatSize(getFileSize(item))}</span>
        {/if}
      </div>
    </button>
  {/each}
  {#if dropIndex !== null && dropIndex >= items.length}
    <div class="drop-gap"></div>
  {/if}
</div>

<style>
  .file-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(140px, 1fr));
    gap: 1rem;
    padding: 0.5rem;
  }

  .file-item {
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: 1rem;
    background: white;
    border: 1px solid #e2e8f0;
    border-radius: 0.75rem;
    cursor: pointer;
    transition: transform 0.15s, box-shadow 0.15s, border-color 0.15s;
    text-align: center;
  }

  .file-item:hover {
    transform: translateY(-2px);
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.1);
    border-color: #3b82f6;
  }

  .file-item.is-folder:hover {
    border-color: #f59e0b;
  }

  .file-item.drag-over {
    border-color: #3b82f6;
    border-style: dashed;
    border-width: 2px;
    background: #eff6ff;
    transform: scale(1.02);
  }

  .file-item.move-into {
    background: #fef3c7;
    border-color: #f59e0b;
    border-width: 2px;
    transform: scale(1.05);
  }

  .file-item.dragging {
    opacity: 0.3;
  }

  .drop-gap {
    min-width: 140px;
    min-height: 120px;
    background: #eff6ff;
    border: 2px dashed #3b82f6;
    border-radius: 0.75rem;
  }

  .file-icon {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 56px;
    height: 56px;
    margin-bottom: 0.75rem;
    color: #64748b;
    background: #f1f5f9;
    border-radius: 0.5rem;
  }

  .file-icon.folder {
    color: #f59e0b;
    background: #fef3c7;
  }

  .file-icon.has-thumbnail {
    width: 100%;
    height: 80px;
    background: transparent;
    overflow: hidden;
    position: relative;
  }

  .file-icon img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    border-radius: 0.5rem;
  }

  .file-type-badge {
    position: absolute;
    bottom: 4px;
    right: 4px;
    background: rgba(255, 255, 255, 0.9);
    border-radius: 4px;
    padding: 2px 4px;
    display: flex;
    align-items: center;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.2);
    color: #64748b;
  }

  .file-info {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.25rem;
    width: 100%;
  }

  .file-name {
    font-size: 0.875rem;
    font-weight: 500;
    color: #1e293b;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .file-size {
    font-size: 0.75rem;
    color: #94a3b8;
  }
</style>
