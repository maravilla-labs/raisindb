/**
 * Sync configuration handling for .raisin-sync.yaml
 */

import fs from 'fs';
import path from 'path';
import yaml from 'yaml';

/**
 * Sync configuration stored in .raisin-sync.yaml
 */
export interface SyncConfig {
  version: number;
  server: string;
  repository: string;
  branch: string;
  remote_path: string;
  conflict_strategy: 'prompt' | 'local' | 'server' | 'newest';
  ignore: string[];
  last_sync?: {
    timestamp: string;
    local_hash: string;
    server_revision: string;
  };
}

const SYNC_CONFIG_FILENAME = '.raisin-sync.yaml';
const DEFAULT_CONFIG: Partial<SyncConfig> = {
  version: 1,
  branch: 'main',
  conflict_strategy: 'prompt',
  ignore: [
    '*.local.*',
    '.raisin-sync.yaml',
    'node_modules/',
    '.git/',
  ],
};

/**
 * Find sync config file by searching up the directory tree
 */
export function findSyncConfig(startDir: string): string | null {
  let currentDir = path.resolve(startDir);
  const root = path.parse(currentDir).root;

  while (currentDir !== root) {
    const configPath = path.join(currentDir, SYNC_CONFIG_FILENAME);
    if (fs.existsSync(configPath)) {
      return configPath;
    }
    currentDir = path.dirname(currentDir);
  }

  return null;
}

/**
 * Load sync config from a directory
 */
export function loadSyncConfig(directory: string): SyncConfig | null {
  const configPath = findSyncConfig(directory);
  if (!configPath) {
    return null;
  }

  try {
    const content = fs.readFileSync(configPath, 'utf-8');
    const config = yaml.parse(content) as SyncConfig;
    return {
      ...DEFAULT_CONFIG,
      ...config,
    } as SyncConfig;
  } catch (error) {
    console.error(`Error loading sync config: ${error}`);
    return null;
  }
}

/**
 * Save sync config to a directory
 */
export function saveSyncConfig(directory: string, config: SyncConfig): void {
  const configPath = path.join(directory, SYNC_CONFIG_FILENAME);
  const content = yaml.stringify(config);
  fs.writeFileSync(configPath, content, 'utf-8');
}

/**
 * Create a new sync config with defaults
 * Note: workspace is now derived from the local file structure (content/{workspace}/...)
 */
export function createSyncConfig(
  server: string,
  repository: string,
  remotePath: string,
  branch: string = 'main'
): SyncConfig {
  return {
    ...DEFAULT_CONFIG,
    version: 1,
    server,
    repository,
    branch,
    remote_path: remotePath,
    conflict_strategy: 'prompt',
    ignore: DEFAULT_CONFIG.ignore || [],
  } as SyncConfig;
}

/**
 * Update last sync state
 */
export function updateLastSync(
  directory: string,
  localHash: string,
  serverRevision: string
): void {
  const config = loadSyncConfig(directory);
  if (!config) {
    throw new Error('No sync config found');
  }

  config.last_sync = {
    timestamp: new Date().toISOString(),
    local_hash: localHash,
    server_revision: serverRevision,
  };

  saveSyncConfig(directory, config);
}

/**
 * Check if a path should be ignored based on config
 */
export function shouldIgnore(config: SyncConfig, relativePath: string): boolean {
  for (const pattern of config.ignore) {
    // Simple pattern matching (supports * and **)
    const regex = patternToRegex(pattern);
    if (regex.test(relativePath)) {
      return true;
    }
  }
  return false;
}

/**
 * Convert a glob pattern to a regex
 */
function patternToRegex(pattern: string): RegExp {
  // Escape special regex characters except * and ?
  let regex = pattern
    .replace(/[.+^${}()|[\]\\]/g, '\\$&')
    .replace(/\*\*/g, '{{GLOBSTAR}}')
    .replace(/\*/g, '[^/]*')
    .replace(/\?/g, '[^/]')
    .replace(/{{GLOBSTAR}}/g, '.*');

  // Handle trailing slash for directories
  if (pattern.endsWith('/')) {
    regex = regex.slice(0, -1) + '(/.*)?';
  }

  return new RegExp(`^${regex}$`);
}

/**
 * Get package directory (directory containing manifest.yaml)
 */
export function findPackageDirectory(startDir: string): string | null {
  let currentDir = path.resolve(startDir);
  const root = path.parse(currentDir).root;

  while (currentDir !== root) {
    const manifestPath = path.join(currentDir, 'manifest.yaml');
    const manifestYmlPath = path.join(currentDir, 'manifest.yml');
    if (fs.existsSync(manifestPath) || fs.existsSync(manifestYmlPath)) {
      return currentDir;
    }
    currentDir = path.dirname(currentDir);
  }

  return null;
}
