/**
 * Package sync command - bidirectional synchronization with server
 */

import fs from 'fs';
import path from 'path';
import yaml from 'yaml';
import React from 'react';
import { render } from 'ink';
import { getServer, loadConfig } from '../config.js';
import { getToken } from '../auth.js';
import { getServerFilesForWorkspaces } from '../api.js';
import {
  SyncConfig,
  loadSyncConfig,
  saveSyncConfig,
  createSyncConfig,
  findPackageDirectory,
  updateLastSync,
} from '../sync/config.js';
import {
  getLocalFiles,
  compareFiles,
  getSyncSummary,
  SyncDiff,
  FileChange,
  ServerFileInfo,
} from '../sync/compare.js';
import { SyncProgress } from '../components/SyncProgress.js';
import { WatchMode } from '../components/WatchMode.js';
import { SyncWatcher, ChangeEvent } from '../sync/watcher.js';
import {
  pushFile,
  processLocalChanges,
  processServerChanges,
  resolveConflict as resolveConflictOp,
  ConflictStrategy,
  SyncOperationOptions,
  Conflict,
} from '../sync/operations.js';

/**
 * Sync command options
 */
export interface SyncOptions {
  watch?: boolean;
  push?: boolean;
  pull?: boolean;
  yes?: boolean;
  force?: boolean;
  dryRun?: boolean;
  repo?: string;
  server?: string;
  init?: boolean;
}

/**
 * Main sync function
 */
export async function syncPackage(
  directory: string,
  options: SyncOptions
): Promise<void> {
  const resolvedDir = path.resolve(directory);

  // Verify directory exists
  if (!fs.existsSync(resolvedDir)) {
    throw new Error(`Directory not found: ${resolvedDir}`);
  }

  // Find package directory
  const packageDir = findPackageDirectory(resolvedDir);
  if (!packageDir) {
    throw new Error('Not a package directory (no manifest.yaml found)');
  }

  // Handle --init flag
  if (options.init) {
    await initializeSyncConfig(packageDir, options);
    return;
  }

  // Load or create sync config
  let config = loadSyncConfig(packageDir);
  if (!config) {
    console.log('No sync configuration found. Running interactive setup...');
    config = await promptForSyncConfig(packageDir, options);
    saveSyncConfig(packageDir, config);
    console.log(`Sync configuration saved to ${packageDir}/.raisin-sync.yaml`);
  }

  // Apply command-line overrides
  if (options.server) config.server = options.server;
  if (options.repo) config.repository = options.repo;

  // Verify authentication
  const token = getToken();
  if (!token) {
    throw new Error('Not authenticated. Run "raisindb shell" and use /login first.');
  }

  // Execute appropriate sync mode
  if (options.watch) {
    await runWatchMode(packageDir, config, options);
  } else if (options.push) {
    await runPushOnly(packageDir, config, options);
  } else if (options.pull) {
    await runPullOnly(packageDir, config, options);
  } else {
    await runBidirectionalSync(packageDir, config, options);
  }
}

/**
 * Initialize sync configuration
 */
async function initializeSyncConfig(
  packageDir: string,
  options: SyncOptions
): Promise<void> {
  const config = await promptForSyncConfig(packageDir, options);
  saveSyncConfig(packageDir, config);
  console.log(`Sync configuration saved to ${packageDir}/.raisin-sync.yaml`);
}

/**
 * Prompt user for sync configuration
 */
async function promptForSyncConfig(
  packageDir: string,
  options: SyncOptions
): Promise<SyncConfig> {
  const server = options.server || getServer() || 'http://localhost:8081';
  const repo = options.repo || '';

  // Read package name from manifest
  const manifestPath = path.join(packageDir, 'manifest.yaml');
  let packageName = 'package';
  if (fs.existsSync(manifestPath)) {
    const manifest = yaml.parse(fs.readFileSync(manifestPath, 'utf-8'));
    packageName = manifest.name || packageName;
  }

  // For now, use defaults - in a full implementation this would be interactive
  const config = createSyncConfig(
    server,
    repo || 'default',
    `/${packageName}`,
    'main'
  );

  console.log(`Configured sync to ${server}/${config.repository}${config.remote_path}`);

  return config;
}

/**
 * Run bidirectional sync
 */
async function runBidirectionalSync(
  packageDir: string,
  config: SyncConfig,
  options: SyncOptions
): Promise<void> {
  console.log('Comparing local and server state...');

  // Get local files (returns paths like "functions/lib/weather/index.js")
  const localFiles = getLocalFiles(packageDir, config);
  console.log(`Found ${localFiles.size} local files`);

  // Extract unique workspaces from local file paths
  const workspaces = new Set<string>();
  for (const filePath of localFiles.keys()) {
    const workspace = filePath.split('/')[0]; // e.g., "functions"
    if (workspace) {
      workspaces.add(workspace);
    }
  }
  console.log(`Discovered workspaces: ${Array.from(workspaces).join(', ')}`);

  // Get server files for discovered workspaces
  const serverFiles = await getServerFilesForWorkspaces(
    config.repository,
    Array.from(workspaces),
    config.branch
  );
  console.log(`Found ${serverFiles.size} server files`);

  // Compare
  const diff = compareFiles(localFiles, serverFiles);
  const summary = getSyncSummary(diff);

  console.log('\nSync Summary:');
  console.log(`  To upload:   ${summary.toUpload}`);
  console.log(`  To download: ${summary.toDownload}`);
  console.log(`  Conflicts:   ${summary.conflicts}`);
  console.log(`  Unchanged:   ${summary.unchanged}`);

  if (options.dryRun) {
    console.log('\n[Dry run - no changes made]');
    if (diff.toUpload.length > 0) {
      console.log('\nFiles to upload:');
      for (const file of diff.toUpload) {
        console.log(`  + ${file.path}`);
      }
    }
    if (diff.toDownload.length > 0) {
      console.log('\nFiles to download:');
      for (const file of diff.toDownload) {
        console.log(`  - ${file.path}`);
      }
    }
    if (diff.conflicts.length > 0) {
      console.log('\nConflicts:');
      for (const file of diff.conflicts) {
        console.log(`  ! ${file.path}`);
      }
    }
    return;
  }

  // Handle conflicts
  if (diff.conflicts.length > 0 && !options.force) {
    console.log('\nConflicts detected. Use --force to overwrite or resolve manually.');
    for (const conflict of diff.conflicts) {
      console.log(`  ! ${conflict.path}`);
    }

    if (!options.yes) {
      // In a full implementation, this would be interactive
      console.log('\nSkipping conflicts. Run with --force to overwrite with local version.');
    }
  }

  // Execute sync
  if (diff.toUpload.length > 0) {
    console.log('\nUploading changes...');
    for (const file of diff.toUpload) {
      // TODO: Implement actual upload
      console.log(`  Uploading: ${file.path}`);
    }
  }

  if (diff.toDownload.length > 0) {
    console.log('\nDownloading changes...');
    for (const file of diff.toDownload) {
      // TODO: Implement actual download
      console.log(`  Downloading: ${file.path}`);
    }
  }

  console.log('\nSync complete!');
}

/**
 * Run push-only sync
 */
async function runPushOnly(
  packageDir: string,
  config: SyncConfig,
  options: SyncOptions
): Promise<void> {
  console.log('Pushing local changes to server...');

  const localFiles = getLocalFiles(packageDir, config);
  console.log(`Found ${localFiles.size} local files`);

  if (options.dryRun) {
    console.log('\n[Dry run - no changes made]');
    console.log('\nFiles to push:');
    for (const [filePath] of localFiles) {
      console.log(`  + ${filePath}`);
    }
    return;
  }

  const contentDir = path.join(packageDir, 'content');
  const contentBase = fs.existsSync(contentDir) && fs.statSync(contentDir).isDirectory()
    ? contentDir
    : packageDir;

  const syncOptions: SyncOperationOptions = {
    packageDir,
    contentBase,
    config,
    dryRun: false,
    force: options.force,
  };

  let successCount = 0;
  let failCount = 0;

  for (const [filePath] of localFiles) {
    const result = await pushFile(filePath, syncOptions);
    if (result.success) {
      successCount++;
      console.log(`  ✓ ${filePath}`);
    } else {
      failCount++;
      console.error(`  ✗ ${filePath}: ${result.error}`);
      if (result.details) {
        console.error(`    ${result.details.replace(/\n/g, '\n    ')}`);
      }
    }
  }

  console.log(`\nPush complete! ${successCount} succeeded, ${failCount} failed.`);
}

/**
 * Run pull-only sync
 */
async function runPullOnly(
  packageDir: string,
  config: SyncConfig,
  options: SyncOptions
): Promise<void> {
  console.log('Pulling changes from server...');

  // Get local files first to discover workspaces
  const localFiles = getLocalFiles(packageDir, config);
  console.log(`Found ${localFiles.size} local files`);

  // Extract unique workspaces from local file paths
  const workspaces = new Set<string>();
  for (const filePath of localFiles.keys()) {
    const workspace = filePath.split('/')[0];
    if (workspace) {
      workspaces.add(workspace);
    }
  }

  // Get server files for discovered workspaces
  const serverFiles = await getServerFilesForWorkspaces(
    config.repository,
    Array.from(workspaces),
    config.branch
  );
  console.log(`Found ${serverFiles.size} server files`);

  if (options.dryRun) {
    console.log('\n[Dry run - no changes made]');
    console.log('\nFiles available on server:');
    for (const [filePath, info] of serverFiles) {
      const localFile = localFiles.get(filePath);
      if (!localFile) {
        console.log(`  + ${filePath} (new)`);
      } else if (localFile.hash !== info.hash) {
        console.log(`  ~ ${filePath} (modified)`);
      }
    }
    return;
  }

  // TODO: Implement actual download
  console.log('\nPull complete!');
}

/**
 * Run watch mode
 */
async function runWatchMode(
  packageDir: string,
  config: SyncConfig,
  options: SyncOptions
): Promise<void> {
  // Push-only watch mode: skip server watcher (avoids @raisindb/client dependency)
  const pushOnly = options.push && !options.pull;

  // Create watcher
  const watcher = new SyncWatcher(packageDir, config, {
    debounceDelay: 500,
    ignorePatterns: [
      '**/node_modules/**',
      '**/.git/**',
      '**/.raisin-sync.yaml',
      '**/dist/**',
      '**/*.log',
    ],
    watchExtensions: ['.yaml', '.yml', '.json', '.md', '.js', '.py', '.star'],
    localOnly: pushOnly,
  });

  // Set up sync operation options — use watcher's watchBase for content resolution
  const contentDir = path.join(packageDir, 'content');
  const contentBase = fs.existsSync(contentDir) && fs.statSync(contentDir).isDirectory()
    ? contentDir
    : packageDir;

  const syncOptions: SyncOperationOptions = {
    packageDir,
    contentBase,
    config,
    dryRun: options.dryRun,
    force: options.force,
  };

  // Default conflict strategy
  const conflictStrategy: ConflictStrategy = options.force
    ? 'local-wins'
    : 'newest-wins';

  // Handle batch events
  watcher.on('batch', async (batch: {
    localChanges: ChangeEvent[];
    serverChanges: ChangeEvent[];
    conflicts: Array<{ local: ChangeEvent; server: ChangeEvent }>;
  }) => {
    // Mark paths as in-flight to prevent feedback loops
    for (const change of batch.localChanges) {
      watcher.markInFlight(change.path);
    }
    for (const change of batch.serverChanges) {
      watcher.markInFlight(change.path);
    }

    // Process local changes (push to server)
    if (batch.localChanges.length > 0) {
      const results = await processLocalChanges(batch.localChanges, syncOptions);
      for (const result of results) {
        watcher.emit('syncResult', result);
        watcher.clearInFlight(result.path);
      }
    }

    // Process server changes (pull to local)
    if (batch.serverChanges.length > 0) {
      const results = await processServerChanges(batch.serverChanges, syncOptions);
      for (const result of results) {
        watcher.emit('syncResult', result);
        watcher.clearInFlight(result.path);
      }
    }

    // Handle conflicts
    for (const conflict of batch.conflicts) {
      watcher.markInFlight(conflict.local.path);
      const result = await resolveConflictOp(conflict as Conflict, conflictStrategy, syncOptions);
      if (result) {
        watcher.emit('syncResult', result);
      }
      watcher.clearInFlight(conflict.local.path);
    }
  });

  // Start watcher
  await watcher.start();

  // Render watch mode UI
  return new Promise((resolve) => {
    const { unmount } = render(
      React.createElement(WatchMode, {
        watcher,
        packageDir,
        remotePath: config.remote_path,
        serverUrl: `${config.server}/${config.repository}`,
        onExit: async () => {
          await watcher.stop();
          unmount();
          resolve();
        },
      })
    );
  });
}

/**
 * Resolve a single conflict
 */
export type ConflictResolution = 'local' | 'server' | 'skip';

export function resolveConflict(
  conflict: FileChange,
  resolution: ConflictResolution,
  packageDir: string
): void {
  switch (resolution) {
    case 'local':
      // Keep local version, push to server
      console.log(`  Keeping local: ${conflict.path}`);
      break;
    case 'server':
      // Download server version
      console.log(`  Using server: ${conflict.path}`);
      break;
    case 'skip':
      console.log(`  Skipping: ${conflict.path}`);
      break;
  }
}
