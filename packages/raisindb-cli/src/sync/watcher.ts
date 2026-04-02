/**
 * File watcher for bidirectional sync
 * Combines chokidar (local) + raisin-client-js (server)
 */

import { watch, type FSWatcher } from 'chokidar';
import path from 'path';
import fs from 'fs';
import { EventEmitter } from 'events';
import { SyncConfig } from './config.js';
import { getToken } from '../auth.js';

// Types for raisin-client (dynamic import)
interface RaisinSubscription {
  id: string;
  unsubscribe(): Promise<void>;
  isActive(): boolean;
}

interface RaisinEventMessage {
  event_type: string;
  subscription_id: string;
  payload: unknown;
  timestamp: number;
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type RaisinClientType = any;

/**
 * Change event from either local or server
 */
export interface ChangeEvent {
  source: 'local' | 'server';
  type: 'add' | 'change' | 'unlink' | 'addDir' | 'unlinkDir';
  path: string;
  timestamp: number;
  nodeId?: string;
  workspace?: string;
}

/**
 * Watcher status
 */
export interface WatcherStatus {
  connected: boolean;
  localWatching: boolean;
  serverSubscribed: boolean;
  lastLocalChange?: number;
  lastServerChange?: number;
  pendingChanges: number;
}

/**
 * Options for the watcher
 */
export interface WatcherOptions {
  /** Debounce delay in ms (default: 500) */
  debounceDelay?: number;
  /** Ignore patterns */
  ignorePatterns?: string[];
  /** File extensions to watch */
  watchExtensions?: string[];
  /** Skip server watcher (push-only mode) */
  localOnly?: boolean;
}

/**
 * Bidirectional file watcher
 * Watches local filesystem with chokidar and server events via WebSocket
 */
export class SyncWatcher extends EventEmitter {
  private packageDir: string;
  private watchBase: string;
  private config: SyncConfig;
  private options: Required<WatcherOptions> & { localOnly: boolean };

  private localWatcher: FSWatcher | null = null;
  private serverClient: RaisinClientType | null = null;
  private serverSubscription: RaisinSubscription | null = null;

  private pendingLocalChanges: Map<string, ChangeEvent> = new Map();
  private pendingServerChanges: Map<string, ChangeEvent> = new Map();
  private debounceTimer: NodeJS.Timeout | null = null;

  private inFlightPaths: Set<string> = new Set();
  private status: WatcherStatus = {
    connected: false,
    localWatching: false,
    serverSubscribed: false,
    pendingChanges: 0,
  };

  constructor(
    packageDir: string,
    config: SyncConfig,
    options: WatcherOptions = {}
  ) {
    super();
    this.packageDir = packageDir;
    // Match getLocalFiles() behavior: if content/ exists, watch from there
    // so relative paths align (e.g. "functions/lib/..." not "content/functions/lib/...")
    const contentDir = path.join(packageDir, 'content');
    this.watchBase = fs.existsSync(contentDir) && fs.statSync(contentDir).isDirectory()
      ? contentDir
      : packageDir;
    this.config = config;
    this.options = {
      debounceDelay: options.debounceDelay ?? 500,
      ignorePatterns: options.ignorePatterns ?? [
        '**/node_modules/**',
        '**/.git/**',
        '**/.raisin-sync.yaml',
        '**/dist/**',
        '**/*.log',
      ],
      watchExtensions: options.watchExtensions ?? ['.yaml', '.yml', '.json', '.md'],
      localOnly: options.localOnly ?? false,
    };
  }

  /**
   * Start watching both local and server
   */
  async start(): Promise<void> {
    if (this.options.localOnly) {
      await this.startLocalWatcher();
    } else {
      await Promise.all([
        this.startLocalWatcher(),
        this.startServerWatcher(),
      ]);
    }

    this.emit('status', this.status);
  }

  /**
   * Stop all watchers
   */
  async stop(): Promise<void> {
    // Stop local watcher
    if (this.localWatcher) {
      await this.localWatcher.close();
      this.localWatcher = null;
      this.status.localWatching = false;
    }

    // Unsubscribe from server events
    if (this.serverSubscription) {
      await this.serverSubscription.unsubscribe();
      this.serverSubscription = null;
      this.status.serverSubscribed = false;
    }

    // Disconnect client
    if (this.serverClient) {
      this.serverClient.disconnect();
      this.serverClient = null;
      this.status.connected = false;
    }

    // Clear pending changes
    if (this.debounceTimer) {
      clearTimeout(this.debounceTimer);
      this.debounceTimer = null;
    }
    this.pendingLocalChanges.clear();
    this.pendingServerChanges.clear();
    this.status.pendingChanges = 0;

    this.emit('status', this.status);
    this.emit('stopped');
  }

  /**
   * Get current status
   */
  getStatus(): WatcherStatus {
    return { ...this.status };
  }

  /**
   * Mark a path as in-flight (being synced)
   */
  markInFlight(filePath: string): void {
    this.inFlightPaths.add(filePath);
  }

  /**
   * Clear in-flight status for a path
   */
  clearInFlight(filePath: string): void {
    this.inFlightPaths.delete(filePath);
  }

  /**
   * Start local file watcher with chokidar
   */
  private async startLocalWatcher(): Promise<void> {
    const watchPath = path.join(this.watchBase, '**/*');

    this.localWatcher = watch(watchPath, {
      ignored: this.options.ignorePatterns,
      persistent: true,
      ignoreInitial: true,
      awaitWriteFinish: {
        stabilityThreshold: 200,
        pollInterval: 100,
      },
    });

    this.localWatcher
      .on('add', (filePath: string) => this.handleLocalChange('add', filePath))
      .on('change', (filePath: string) => this.handleLocalChange('change', filePath))
      .on('unlink', (filePath: string) => this.handleLocalChange('unlink', filePath))
      .on('addDir', (filePath: string) => this.handleLocalChange('addDir', filePath))
      .on('unlinkDir', (filePath: string) => this.handleLocalChange('unlinkDir', filePath))
      .on('error', (error: Error) => {
        this.emit('error', new Error(`Local watcher error: ${error.message}`));
      })
      .on('ready', () => {
        this.status.localWatching = true;
        this.emit('status', this.status);
        this.emit('localReady');
      });
  }

  /**
   * Start server event watcher via WebSocket
   */
  private async startServerWatcher(): Promise<void> {
    const token = getToken();
    if (!token) {
      this.emit('error', new Error('Not authenticated. Cannot subscribe to server events.'));
      return;
    }

    try {
      // Dynamic import of @raisindb/client
      const { RaisinClient } = await import('@raisindb/client');

      // Build raisin:// URL from HTTP server URL
      // RaisinClient accepts raisin:// and internally maps to ws://
      const parsed = new URL(this.config.server);
      const raisinProtocol = parsed.protocol === 'https:' ? 'raisins://' : 'raisin://';
      const raisinUrl = `${raisinProtocol}${parsed.host}/sys/${this.config.repository}`;

      this.serverClient = new RaisinClient(raisinUrl);

      // Set up connection handlers
      this.serverClient.on('authenticated', () => {
        this.status.connected = true;
        this.emit('status', this.status);
        this.emit('serverConnected');
      });

      // Connect and authenticate
      await this.serverClient.connect();
      await this.serverClient.authenticate({
        accessToken: token,
      });

      // Subscribe to package events
      const db = this.serverClient.database(this.config.repository);
      const events = db.events();

      this.serverSubscription = await events.subscribe(
        {
          workspace: 'packages',
          path: `${this.config.remote_path}/**`,
          event_types: ['node:created', 'node:updated', 'node:deleted'],
        },
        (event: RaisinEventMessage) => this.handleServerEvent(event)
      ) as RaisinSubscription;

      this.status.serverSubscribed = true;
      this.emit('status', this.status);
      this.emit('serverSubscribed');
    } catch (error) {
      this.emit('error', new Error(`Failed to connect to server: ${error instanceof Error ? error.message : 'Unknown error'}`));
    }
  }

  /**
   * Handle local file change
   */
  private handleLocalChange(type: ChangeEvent['type'], filePath: string): void {
    // Get relative path from watch base (content/ dir if it exists)
    const relativePath = path.relative(this.watchBase, filePath).split(path.sep).join('/');

    // Ignore changes to in-flight paths (currently being synced)
    if (this.inFlightPaths.has(relativePath)) {
      return;
    }

    // Check if file extension is watched
    const ext = path.extname(filePath);
    if (
      this.options.watchExtensions.length > 0 &&
      !this.options.watchExtensions.includes(ext) &&
      type !== 'addDir' &&
      type !== 'unlinkDir'
    ) {
      return;
    }

    const event: ChangeEvent = {
      source: 'local',
      type,
      path: relativePath,
      timestamp: Date.now(),
    };

    this.pendingLocalChanges.set(relativePath, event);
    this.status.lastLocalChange = event.timestamp;
    this.status.pendingChanges = this.pendingLocalChanges.size + this.pendingServerChanges.size;

    this.emit('localChange', event);
    this.emit('status', this.status);

    // Debounce batch processing
    this.scheduleBatch();
  }

  /**
   * Handle server event
   */
  private handleServerEvent(event: RaisinEventMessage): void {
    const payload = event.payload as Record<string, unknown>;
    const nodePath = payload.path as string | undefined;
    const nodeId = payload.node_id as string | undefined;
    const workspace = payload.workspace as string | undefined;

    if (!nodePath) return;

    // Ignore changes to in-flight paths
    if (this.inFlightPaths.has(nodePath)) {
      return;
    }

    let type: ChangeEvent['type'];
    switch (event.event_type) {
      case 'node:created':
        type = 'add';
        break;
      case 'node:updated':
        type = 'change';
        break;
      case 'node:deleted':
        type = 'unlink';
        break;
      default:
        return;
    }

    const changeEvent: ChangeEvent = {
      source: 'server',
      type,
      path: nodePath,
      timestamp: Date.now(),
      nodeId,
      workspace,
    };

    this.pendingServerChanges.set(nodePath, changeEvent);
    this.status.lastServerChange = changeEvent.timestamp;
    this.status.pendingChanges = this.pendingLocalChanges.size + this.pendingServerChanges.size;

    this.emit('serverChange', changeEvent);
    this.emit('status', this.status);

    // Debounce batch processing
    this.scheduleBatch();
  }

  /**
   * Schedule batch processing of changes
   */
  private scheduleBatch(): void {
    if (this.debounceTimer) {
      clearTimeout(this.debounceTimer);
    }

    this.debounceTimer = setTimeout(() => {
      this.processBatch();
    }, this.options.debounceDelay);
  }

  /**
   * Process batched changes
   */
  private processBatch(): void {
    const localChanges = Array.from(this.pendingLocalChanges.values());
    const serverChanges = Array.from(this.pendingServerChanges.values());

    // Clear pending changes
    this.pendingLocalChanges.clear();
    this.pendingServerChanges.clear();
    this.status.pendingChanges = 0;

    // Detect conflicts (same path changed on both sides)
    const conflicts: Array<{ local: ChangeEvent; server: ChangeEvent }> = [];
    const localOnly: ChangeEvent[] = [];
    const serverOnly: ChangeEvent[] = [];

    const serverByPath = new Map(serverChanges.map((e) => [e.path, e]));

    for (const local of localChanges) {
      const server = serverByPath.get(local.path);
      if (server) {
        conflicts.push({ local, server });
        serverByPath.delete(local.path);
      } else {
        localOnly.push(local);
      }
    }

    serverOnly.push(...serverByPath.values());

    // Emit batch event for processing
    this.emit('batch', {
      localChanges: localOnly,
      serverChanges: serverOnly,
      conflicts,
    });

    this.emit('status', this.status);
  }
}
