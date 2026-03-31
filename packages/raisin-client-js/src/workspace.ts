/**
 * Workspace operations interface
 */

import { NodeOperations } from './nodes';
import { EventSubscriptions, EventHandler } from './events';
import { Transaction } from './transactions';
import {
  Workspace,
  WorkspaceCreatePayload,
  WorkspaceGetPayload,
  WorkspaceDeletePayload,
  WorkspaceUpdatePayload,
  RequestContext,
} from './protocol';
import { logger } from './logger';
import type { Upload, UploadOptions, BatchUpload, BatchUploadOptions } from './upload/types';
import type { UploadManager } from './upload/uploader';
import { createFileSource } from './upload/file-source';
import type { SignAssetOptions, SignedAssetUrl } from './http-client';

/**
 * Options for creating a workspace
 */
export interface WorkspaceCreateOptions {
  /** Workspace name */
  name: string;
  /** Workspace description */
  description?: string;
}

/**
 * Options for updating a workspace
 */
export interface WorkspaceUpdateOptions {
  /** Workspace description */
  description?: string;
  /** Allowed node types */
  allowed_node_types?: string[];
  /** Allowed root node types */
  allowed_root_node_types?: string[];
}

/**
 * Workspace interface for content operations
 */
export class WorkspaceClient {
  private name: string;
  private _context: RequestContext;
  private sendRequest: (payload: unknown, requestType?: string) => Promise<unknown>;
  private eventHandler: EventHandler;
  private nodeOps?: NodeOperations;
  private eventSubs?: EventSubscriptions;
  private branch?: string;
  private revision?: string;
  private getUploadManager?: () => UploadManager;
  private getSignAssetUrl?: (options: SignAssetOptions) => Promise<SignedAssetUrl>;

  constructor(
    name: string,
    context: RequestContext,
    sendRequest: (payload: unknown, requestType?: string, contextOverride?: RequestContext) => Promise<unknown>,
    eventHandler: EventHandler,
    branch?: string,
    revision?: string,
    getUploadManager?: () => UploadManager,
    getSignAssetUrl?: (options: SignAssetOptions) => Promise<SignedAssetUrl>
  ) {
    this.name = name;
    this.branch = branch;
    this.revision = revision;
    // Store context with workspace set
    this._context = {
      ...context,
      workspace: name,
      branch: branch || context.branch,
      revision: revision || context.revision
    };
    // Wrap sendRequest to always include our workspace context
    this.sendRequest = (payload: unknown, requestType?: string) => {
      logger.debug('WorkspaceClient sendRequest wrapper called');
      logger.debug('WorkspaceClient workspace context:', JSON.stringify(this._context, null, 2));
      return sendRequest(payload, requestType, this._context);
    };
    this.eventHandler = eventHandler;
    this.getUploadManager = getUploadManager;
    this.getSignAssetUrl = getSignAssetUrl;
  }

  /**
   * Get the workspace name
   */
  getName(): string {
    return this.name;
  }

  /**
   * Create a new WorkspaceClient instance scoped to a specific branch.
   *
   * @param branch - Branch name
   * @returns New WorkspaceClient instance with branch context
   */
  onBranch(branch: string): WorkspaceClient {
    return new WorkspaceClient(
      this.name,
      this._context,
      this.sendRequest,
      this.eventHandler,
      branch,
      this.revision,
      this.getUploadManager,
      this.getSignAssetUrl
    );
  }

  /**
   * Create a new WorkspaceClient instance scoped to a specific revision/commit.
   *
   * @param revision - Revision/commit ID
   * @returns New WorkspaceClient instance with revision context
   */
  atRevision(revision: string): WorkspaceClient {
    return new WorkspaceClient(
      this.name,
      this._context,
      this.sendRequest,
      this.eventHandler,
      this.branch,
      revision,
      this.getUploadManager,
      this.getSignAssetUrl
    );
  }

  /**
   * Get node operations for this workspace
   */
  nodes(): NodeOperations {
    if (!this.nodeOps) {
      this.nodeOps = new NodeOperations(
        (payload, requestType?) =>
          this.sendRequest(payload, requestType || this.inferRequestType(payload)),
        this.name
      );
    }
    return this.nodeOps;
  }

  /**
   * Get event subscriptions for this workspace
   */
  events(): EventSubscriptions {
    if (!this.eventSubs) {
      this.eventSubs = new EventSubscriptions(this.eventHandler, {
        workspace: this.name,
      });
    }
    return this.eventSubs;
  }

  /**
   * Create a new transaction for this workspace
   *
   * @returns Transaction instance
   *
   * @example
   * ```typescript
   * const tx = ws.transaction();
   * await tx.begin({ message: 'Create initial content' });
   *
   * try {
   *   await tx.nodes().create({ type: 'Page', path: '/home', properties: { title: 'Home' } });
   *   await tx.nodes().create({ type: 'Page', path: '/about', properties: { title: 'About' } });
   *   await tx.commit();
   * } catch (error) {
   *   await tx.rollback();
   *   throw error;
   * }
   * ```
   */
  transaction(): Transaction {
    // Need to pass the raw sendRequest that accepts contextOverride
    // We can't use this.sendRequest because it always injects workspace context
    // Transaction will handle context management

    // Extract the original sendRequest from the constructor parameter
    // by creating a new wrapper that bypasses our workspace context injection
    const rawSendRequest = (payload: unknown, requestType: string, _contextOverride?: RequestContext) => {
      // Use the sendRequest but respect the contextOverride from Transaction
      return this.sendRequest(payload, requestType);
    };

    return new Transaction(this._context, rawSendRequest);
  }

  // ============================================================================
  // Upload Methods (convenience methods that delegate to UploadManager)
  // ============================================================================

  /**
   * Create a resumable upload for this workspace (Browser)
   *
   * Convenience method that pre-fills repository and workspace context.
   *
   * @param file - File or Blob to upload
   * @param path - Path for the new node
   * @param options - Additional upload options
   * @returns Upload controller
   *
   * @example
   * ```typescript
   * const ws = client.database('media').workspace('assets');
   * const upload = await ws.upload(file, '/videos/my-video.mp4', {
   *   onProgress: (p) => console.log(`${Math.round(p.progress * 100)}%`)
   * });
   * await upload.start();
   * ```
   */
  async upload(
    file: File | Blob,
    path: string,
    options?: Partial<Omit<UploadOptions, 'repository' | 'workspace' | 'path'>>
  ): Promise<Upload> {
    if (!this.getUploadManager) {
      throw new Error('Upload not available: client does not support uploads');
    }

    const fileSource = await createFileSource(file);
    const manager = this.getUploadManager();
    return manager.createUpload(fileSource, {
      ...options,
      repository: this._context.repository!,
      workspace: this.name,
      path,
      branch: options?.branch ?? this._context.branch,
    });
  }

  /**
   * Create a resumable upload for this workspace (Node.js)
   *
   * Convenience method that pre-fills repository and workspace context.
   *
   * @param filePath - Path to the file to upload
   * @param nodePath - Path for the new node
   * @param options - Additional upload options
   * @returns Upload controller
   *
   * @example
   * ```typescript
   * const ws = client.database('data').workspace('backups');
   * const upload = await ws.uploadFile('/path/to/backup.zip', '/backup.zip');
   * await upload.start();
   * ```
   */
  async uploadFile(
    filePath: string,
    nodePath: string,
    options?: Partial<Omit<UploadOptions, 'repository' | 'workspace' | 'path'>>
  ): Promise<Upload> {
    if (!this.getUploadManager) {
      throw new Error('Upload not available: client does not support uploads');
    }

    const fileSource = await createFileSource(filePath);
    const manager = this.getUploadManager();
    return manager.createUpload(fileSource, {
      ...options,
      repository: this._context.repository!,
      workspace: this.name,
      path: nodePath,
      branch: options?.branch ?? this._context.branch,
    });
  }

  /**
   * Upload multiple files to this workspace
   *
   * Creates a batch upload with concurrency control and aggregate progress tracking.
   *
   * @param files - FileList, File[], or Blob[] to upload
   * @param basePath - Base path for all uploads (files go to basePath/filename)
   * @param options - Additional batch upload options
   * @returns BatchUpload controller
   *
   * @example
   * ```typescript
   * const ws = client.database('media').workspace('assets');
   * const files = document.querySelector('input[type="file"]').files;
   * const batch = await ws.uploadFiles(files, '/uploads', {
   *   concurrency: 3,
   *   onProgress: (p) => console.log(`${p.filesCompleted}/${p.filesTotal}`),
   * });
   * const result = await batch.start();
   * ```
   */
  async uploadFiles(
    files: FileList | File[] | Blob[],
    basePath: string,
    options?: Partial<Omit<BatchUploadOptions, 'repository' | 'workspace' | 'basePath'>>
  ): Promise<BatchUpload> {
    if (!this.getUploadManager) {
      throw new Error('Upload not available: client does not support uploads');
    }

    const sources = await Promise.all(
      Array.from(files).map((f) => createFileSource(f))
    );
    const manager = this.getUploadManager();
    return manager.createBatchUpload(sources, {
      ...options,
      repository: this._context.repository!,
      workspace: this.name,
      basePath,
      branch: options?.branch ?? this._context.branch,
    });
  }

  // ============================================================================
  // Asset Signed URL Methods
  // ============================================================================

  /**
   * Get a signed URL for an asset in this workspace
   *
   * Convenience method that pre-fills repository and workspace.
   *
   * @param path - Path to the asset node
   * @param command - 'download' (forces download) or 'display' (shows inline)
   * @param options - Optional settings: expiresIn (seconds), propertyPath (e.g., 'thumbnail')
   * @returns Signed URL and expiry time
   *
   * @example
   * ```typescript
   * const ws = client.database('media').workspace('assets');
   * const { url } = await ws.signAssetUrl('/files/image.jpeg', 'display');
   * document.querySelector('img').src = url;
   *
   * // Get thumbnail instead of main file
   * const { url: thumbUrl } = await ws.signAssetUrl('/files/image.jpeg', 'display', { propertyPath: 'thumbnail' });
   * ```
   */
  async signAssetUrl(
    path: string,
    command: 'download' | 'display' = 'display',
    options?: { expiresIn?: number; propertyPath?: string }
  ): Promise<SignedAssetUrl> {
    if (!this.getSignAssetUrl) {
      throw new Error('SignAssetUrl not available: client does not support it');
    }
    return this.getSignAssetUrl({
      repository: this._context.repository!,
      workspace: this.name,
      path,
      command,
      branch: this._context.branch,
      expiresIn: options?.expiresIn,
      propertyPath: options?.propertyPath,
    });
  }

  /**
   * Infer request type from payload
   * This is a helper to determine the correct RequestType enum value
   */
  private inferRequestType(payload: unknown): string {
    if (typeof payload !== 'object' || payload === null) {
      throw new Error('Invalid payload');
    }

    const keys = Object.keys(payload);

    if ('node_type' in payload && 'path' in payload) {
      return 'node_create';
    }
    if ('node_id' in payload && 'properties' in payload) {
      return 'node_update';
    }
    if ('node_id' in payload && keys.length === 1) {
      if ((payload as any).node_id) {
        return 'node_get';
      }
      return 'node_delete';
    }
    if ('query' in payload) {
      return 'node_query';
    }

    throw new Error('Unable to infer request type from payload');
  }
}

/**
 * Workspace management operations
 */
export class WorkspaceManager {
  private _context: RequestContext;
  private sendRequest: (payload: unknown, requestType?: string, contextOverride?: RequestContext) => Promise<unknown>;
  private eventHandler: EventHandler;
  private getUploadManager?: () => UploadManager;
  private getSignAssetUrl?: (options: SignAssetOptions) => Promise<SignedAssetUrl>;

  constructor(
    context: RequestContext,
    sendRequest: (payload: unknown, requestType?: string, contextOverride?: RequestContext) => Promise<unknown>,
    eventHandler: EventHandler,
    getUploadManager?: () => UploadManager,
    getSignAssetUrl?: (options: SignAssetOptions) => Promise<SignedAssetUrl>
  ) {
    this._context = context;
    this.sendRequest = sendRequest;
    this.eventHandler = eventHandler;
    this.getUploadManager = getUploadManager;
    this.getSignAssetUrl = getSignAssetUrl;
  }

  /**
   * Get a workspace client for operations
   *
   * @param name - Workspace name
   * @returns Workspace client
   */
  workspace(name: string): WorkspaceClient {
    return new WorkspaceClient(
      name,
      this._context,
      this.sendRequest,
      this.eventHandler,
      undefined,
      undefined,
      this.getUploadManager,
      this.getSignAssetUrl
    );
  }

  /**
   * Create a new workspace
   *
   * @param options - Workspace creation options
   * @returns Created workspace
   */
  async create(options: WorkspaceCreateOptions): Promise<Workspace> {
    const payload: WorkspaceCreatePayload = {
      name: options.name,
      description: options.description,
    };

    const result = await this.sendRequest(payload, 'workspace_create');
    return result as Workspace;
  }

  /**
   * Get workspace metadata
   *
   * @param name - Workspace name
   * @returns Workspace metadata
   */
  async get(name: string): Promise<Workspace | null> {
    const payload: WorkspaceGetPayload = {
      name,
    };

    try {
      const result = await this.sendRequest(payload, 'workspace_get');
      return result as Workspace;
    } catch (error) {
      if (error instanceof Error && error.message.includes('not found')) {
        return null;
      }
      throw error;
    }
  }

  /**
   * List all workspaces
   *
   * @returns Array of workspaces
   */
  async list(): Promise<Workspace[]> {
    const result = await this.sendRequest({}, 'workspace_list');
    return result as Workspace[];
  }

  /**
   * Update workspace metadata
   *
   * @param name - Workspace name
   * @param options - Update options
   * @returns Updated workspace
   */
  async update(name: string, options: WorkspaceUpdateOptions): Promise<Workspace> {
    const payload: WorkspaceUpdatePayload = {
      name,
      description: options.description,
      allowed_node_types: options.allowed_node_types,
      allowed_root_node_types: options.allowed_root_node_types,
    };

    const result = await this.sendRequest(payload, 'workspace_update');
    return result as Workspace;
  }

  /**
   * Delete a workspace
   *
   * @param name - Workspace name
   * @returns true if deleted successfully
   */
  async delete(name: string): Promise<boolean> {
    const payload: WorkspaceDeletePayload = {
      name,
    };

    await this.sendRequest(payload, 'workspace_delete');
    return true;
  }
}
