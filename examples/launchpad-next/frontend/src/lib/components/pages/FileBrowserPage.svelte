<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { FolderPlus, Upload, RefreshCw } from 'lucide-svelte';
  import type { PageNode } from '$lib/raisin';
  import { query, getClient, reorderNode, moveNode } from '$lib/raisin';
  import FolderBreadcrumbs from './file-browser/FolderBreadcrumbs.svelte';
  import FileGrid from './file-browser/FileGrid.svelte';
  import FilePreview from './file-browser/FilePreview.svelte';
  import UploadDropZone from './file-browser/UploadDropZone.svelte';
  import CreateFolderDialog from './file-browser/CreateFolderDialog.svelte';

  export interface FileItem {
    id: string;
    name: string;
    path: string;
    node_type: string;
    properties: {
      title?: string;
      description?: string;
      alt_text?: string;
      keywords?: string[];
      caption?: string;
      file?: {
        uuid: string;
        name: string;
        size: number;
        mime_type: string;
        url: string;
      };
      thumbnail?: {
        uuid: string;
        name: string;
        size: number;
        mime_type: string;
        url: string;
      };
      file_type?: string;
      file_size?: number;
      meta?: {
        processing?: {
          progress: number;
          status: string;
        };
      };
    };
  }

  export interface UploadProgressItem {
    id: string;
    filename: string;
    progress: number;
    status: 'uploading' | 'completed' | 'error';
    error?: string;
  }

  interface Props {
    page: PageNode;
  }

  let { page }: Props = $props();

  // Current folder path relative to page.path
  let currentPath = $state('');
  let items = $state<FileItem[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);

  // UI state
  let showCreateFolder = $state(false);
  let showPreview = $state(false);
  let previewItem = $state<FileItem | null>(null);
  let uploads = $state<UploadProgressItem[]>([]);
  let isSyncing = $state(false);

  // Real-time subscription
  let unsubscribe: (() => void) | null = null;

  // Compute full path
  const fullPath = $derived(page.path + currentPath);

  // Load folder contents
  async function loadFolder() {
    loading = true;
    error = null;
    try {
      const result = await query<FileItem>(`
        SELECT id, name, path, node_type, properties
        FROM launchpad
        WHERE CHILD_OF($1)
          AND (node_type = 'raisin:Folder' OR node_type = 'raisin:Asset')
      `, [fullPath]);
      // Group folders first, then files, preserving order within each group
      const folders = result.filter(item => item.node_type === 'raisin:Folder');
      const files = result.filter(item => item.node_type !== 'raisin:Folder');
      items = [...folders, ...files];
    } catch (err) {
      console.error('[file-browser] Failed to load folder:', err);
      error = err instanceof Error ? err.message : String(err);
    } finally {
      loading = false;
    }
  }

  // Navigate to folder
  function navigateToFolder(folderPath: string) {
    // Calculate relative path from page.path
    if (folderPath.startsWith(page.path)) {
      currentPath = folderPath.slice(page.path.length);
    } else {
      currentPath = folderPath;
    }
  }

  // Navigate up via breadcrumb
  function navigateToBreadcrumb(relativePath: string) {
    currentPath = relativePath;
  }

  // Open file preview
  function openPreview(item: FileItem) {
    previewItem = item;
    showPreview = true;
  }

  // Handle file selection (folder -> navigate, file -> preview)
  function handleItemClick(item: FileItem) {
    if (item.node_type === 'raisin:Folder') {
      navigateToFolder(item.path);
    } else {
      openPreview(item);
    }
  }

  // Create folder
  async function handleCreateFolder(name: string) {
    try {
      const folderPath = fullPath + '/' + name;
      await query(`
        INSERT INTO launchpad (path, node_type, properties)
        VALUES ($1, 'raisin:Folder', $2::jsonb)
      `, [folderPath, JSON.stringify({ description: '' })]);
      showCreateFolder = false;
      await loadFolder();
    } catch (err) {
      console.error('[file-browser] Failed to create folder:', err);
      throw err;
    }
  }

  // Handle file uploads to a specific path
  async function uploadFilesToPath(files: File[], targetPath: string) {
    const client = getClient();

    // Create progress entries
    const newUploads: UploadProgressItem[] = files.map(file => ({
      id: crypto.randomUUID(),
      filename: file.name,
      progress: 0,
      status: 'uploading' as const
    }));
    uploads = [...uploads, ...newUploads];

    try {
      const batch = await client.uploadFiles(files, {
        repository: 'launchpad-next',
        workspace: 'launchpad',
        basePath: targetPath,
        concurrency: 3,
        onProgress: (progress) => {
          // Update per-file progress from progress.files[]
          progress.files.forEach((file: { file: string; progress: number; status: string }) => {
            uploads = uploads.map(u =>
              u.filename === file.file
                ? {
                    ...u,
                    progress: Math.round(file.progress * 100),
                    status: file.status === 'failed' ? 'error' as const :
                            file.status === 'completed' ? 'completed' as const : 'uploading' as const
                  }
                : u
            );
          });
        },
        onFileComplete: (filename: string) => {
          uploads = uploads.map(u =>
            u.filename === filename
              ? { ...u, progress: 100, status: 'completed' as const }
              : u
          );
        },
        onFileError: (filename: string, err: Error) => {
          uploads = uploads.map(u =>
            u.filename === filename
              ? { ...u, status: 'error' as const, error: err.message }
              : u
          );
        }
      });

      await batch.start();

      // Refresh folder after all uploads complete
      await loadFolder();

      // Clear completed uploads after delay
      setTimeout(() => {
        uploads = uploads.filter(u => u.status !== 'completed');
      }, 3000);
    } catch (err) {
      console.error('[file-browser] Upload failed:', err);
    }
  }

  // Handle files dropped in current folder (from UploadDropZone)
  function handleFilesDropped(files: File[]) {
    uploadFilesToPath(files, fullPath);
  }

  // Handle files dropped onto a folder (from FileGrid)
  function handleDropToFolder(files: File[], folderPath: string) {
    uploadFilesToPath(files, folderPath);
  }

  // Handle reordering items (from FileGrid)
  async function handleReorder(source: FileItem, target: FileItem, position: 'above' | 'below') {
    try {
      await reorderNode(source.path, target.path, position);
      // Real-time subscription will auto-refresh, but trigger sync indicator
      isSyncing = true;
      setTimeout(() => {
        isSyncing = false;
      }, 500);
    } catch (err) {
      console.error('[file-browser] Failed to reorder:', err);
    }
  }

  // Handle moving item into a folder (from FileGrid)
  async function handleMoveToFolder(source: FileItem, targetFolderPath: string) {
    try {
      await moveNode(source.path, targetFolderPath);
      // Real-time subscription will auto-refresh, but trigger sync indicator
      isSyncing = true;
      setTimeout(() => {
        isSyncing = false;
      }, 500);
    } catch (err) {
      console.error('[file-browser] Failed to move:', err);
    }
  }

  // Reload current folder
  async function refresh() {
    isSyncing = true;
    await loadFolder();
    setTimeout(() => {
      isSyncing = false;
    }, 500);
  }

  // Watch for path changes
  $effect(() => {
    // Re-run when fullPath changes
    const _ = fullPath;
    loadFolder();
  });

  // Subscribe to real-time events
  onMount(async () => {
    try {
      const client = getClient();
      const db = client.database('launchpad-next');
      const workspace = db.workspace('launchpad');
      const events = workspace.events();

      const subscription = await events.subscribe(
        {
          workspace: 'launchpad',
          path: page.path + '/**',
          event_types: ['node:created', 'node:updated', 'node:deleted', 'node:reordered'],
        },
        async () => {
          // Refresh on any change in this subtree
          isSyncing = true;
          await loadFolder();
          setTimeout(() => {
            isSyncing = false;
          }, 500);
        }
      );

      unsubscribe = () => subscription.unsubscribe();
    } catch (err) {
      console.error('[file-browser] Failed to subscribe:', err);
    }
  });

  onDestroy(() => {
    if (unsubscribe) {
      unsubscribe();
    }
  });
</script>

<article class="file-browser">
  <header class="browser-header">
    <div class="header-left">
      <h1>{page.properties.title}</h1>
      {#if page.properties.description}
        <p class="description">{page.properties.description}</p>
      {/if}
    </div>
    <div class="header-right">
      <div class="sync-indicator" class:syncing={isSyncing}>
        <RefreshCw size={16} class={isSyncing ? 'spinning' : ''} />
      </div>
      <button class="action-btn" onclick={() => showCreateFolder = true} title="New folder">
        <FolderPlus size={18} />
        <span>New Folder</span>
      </button>
    </div>
  </header>

  <FolderBreadcrumbs
    baseName={page.properties.title || 'Files'}
    {currentPath}
    onNavigate={navigateToBreadcrumb}
  />

  <UploadDropZone onFilesDropped={handleFilesDropped} {uploads}>
    {#if loading && items.length === 0}
      <div class="loading-state">
        <RefreshCw size={24} class="spinning" />
        <p>Loading files...</p>
      </div>
    {:else if error}
      <div class="error-state">
        <p>Error: {error}</p>
        <button onclick={refresh}>Retry</button>
      </div>
    {:else if items.length === 0}
      <div class="empty-state">
        <Upload size={48} />
        <p>No files yet</p>
        <p class="hint">Drag and drop files here or click upload</p>
      </div>
    {:else}
      <FileGrid {items} onItemClick={handleItemClick} onDropToFolder={handleDropToFolder} onReorder={handleReorder} onMoveToFolder={handleMoveToFolder} />
    {/if}
  </UploadDropZone>

  {#if showCreateFolder}
    <CreateFolderDialog
      onClose={() => showCreateFolder = false}
      onCreate={handleCreateFolder}
    />
  {/if}

  {#if showPreview && previewItem}
    <FilePreview
      item={previewItem}
      onClose={() => { showPreview = false; previewItem = null; }}
    />
  {/if}
</article>

<style>
  .file-browser {
    min-height: 100vh;
    padding: 2rem;
  }

  .browser-header {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    margin-bottom: 1.5rem;
  }

  .header-left h1 {
    font-size: 2rem;
    font-weight: 700;
    font-family: var(--font-display);
    color: var(--color-text-heading);
    margin: 0 0 0.5rem;
  }

  .header-left .description {
    color: var(--color-text-secondary);
    font-family: var(--font-body);
    font-size: 1rem;
    margin: 0;
  }

  .header-right {
    display: flex;
    align-items: center;
    gap: 0.75rem;
  }

  .sync-indicator {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 36px;
    height: 36px;
    color: var(--color-text-muted);
    border-radius: var(--radius-sm);
  }

  .sync-indicator.syncing {
    color: var(--color-accent);
  }

  .sync-indicator :global(.spinning) {
    animation: spin 1s linear infinite;
  }

  @keyframes spin {
    from { transform: rotate(0deg); }
    to { transform: rotate(360deg); }
  }

  .action-btn {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.625rem 1rem;
    background: var(--color-accent);
    color: var(--color-bg);
    border: none;
    border-radius: var(--radius-sm);
    font-size: 0.875rem;
    font-weight: 500;
    font-family: var(--font-body);
    cursor: pointer;
    transition: background-color 0.15s;
  }

  .action-btn:hover {
    background: var(--color-accent-hover);
  }

  .loading-state,
  .error-state,
  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: 4rem 2rem;
    color: var(--color-text-secondary);
    text-align: center;
  }

  .loading-state p,
  .error-state p,
  .empty-state p {
    margin: 1rem 0 0;
    font-size: 1rem;
    font-family: var(--font-body);
  }

  .empty-state .hint {
    font-size: 0.875rem;
    color: var(--color-text-muted);
    margin-top: 0.5rem;
  }

  .error-state button {
    margin-top: 1rem;
    padding: 0.5rem 1rem;
    background: var(--color-accent);
    color: var(--color-bg);
    border: none;
    border-radius: var(--radius-sm);
    font-family: var(--font-body);
    cursor: pointer;
  }
</style>
