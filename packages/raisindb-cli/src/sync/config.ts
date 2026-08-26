/**
 * Sync configuration handling for .raisindb-cli.yaml
 */

import fs from 'fs';
import path from 'path';
import yaml from 'yaml';
import { hasEnvTokens, substituteEnvTokens } from '../env/substitute.js';
import { loadEnvContext, EnvLoadOptions } from '../env/load.js';

/**
 * Sync configuration stored in .raisindb-cli.yaml
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

const SYNC_CONFIG_FILENAME = '.raisindb-cli.yaml';

/**
 * The name this file used to have — still READ, never written.
 *
 * It was renamed because `.raisin-sync.yaml` is ALSO the name of a completely
 * different file: a PACKAGE's install-reconciliation policy, which ships inside
 * the .rap beside `manifest.yaml` and tells `raisin:install` which paths it may
 * overwrite. Same name, unrelated schema. A package that followed the
 * documented convention could not run `sync --push` at all: the CLI loaded the
 * install policy as its own config, found no `server`, and threw
 * `Cannot read properties of undefined (reading 'startsWith')` from
 * `toHttpUrl` — with `--dry-run` succeeding, because planning never builds a
 * URL.
 *
 * The legacy name is therefore accepted ONLY when the file is shaped like a CLI
 * config (see `looksLikeCliConfig`). That check is what makes the collision
 * un-repeatable rather than merely renamed away: an install policy at the old
 * name is now ignored, and the caller falls through to `~/.raisinrc`.
 */
const LEGACY_CONFIG_FILENAME = '.raisin-sync.yaml';

/**
 * Does this parsed document belong to the CLI, or is it a package's install
 * policy that happens to share the legacy filename?
 *
 * `server` is the discriminator because it is the one field the CLI cannot
 * work without and the install policy never has — the policy is `defaults` +
 * `filters`, addressed to the server, which already knows where it is.
 */
function looksLikeCliConfig(doc: unknown): boolean {
  return !!doc && typeof doc === 'object' && typeof (doc as SyncConfig).server === 'string';
}
const DEFAULT_CONFIG: Partial<SyncConfig> = {
  version: 1,
  branch: 'main',
  conflict_strategy: 'prompt',
  ignore: [
    '*.local.*',
    '.raisindb-cli.yaml',
    '.raisin-sync.yaml',
    'node_modules/',
    '.git/',
    // Env files feed {env:...} substitution; their values are pushed, they are not.
    '.env',
    '.env.*',
  ],
};

/** Cheap shape probe for a legacy-named file: parse and look for `server`. */
function isCliConfigFile(configPath: string): boolean {
  try {
    return looksLikeCliConfig(yaml.parse(fs.readFileSync(configPath, 'utf-8')));
  } catch {
    return false;
  }
}

/**
 * Find sync config file by searching up the directory tree
 */
export function findSyncConfig(startDir: string): string | null {
  let currentDir = path.resolve(startDir);
  const root = path.parse(currentDir).root;

  while (currentDir !== root) {
    for (const name of [SYNC_CONFIG_FILENAME, LEGACY_CONFIG_FILENAME]) {
      const configPath = path.join(currentDir, name);
      // A legacy-named file is only OURS if it is shaped like a CLI config;
      // otherwise it is a package install policy and we must not claim it.
      if (fs.existsSync(configPath) && (name === SYNC_CONFIG_FILENAME || isCliConfigFile(configPath))) {
        return configPath;
      }
    }
    currentDir = path.dirname(currentDir);
  }

  return null;
}

/**
 * Load sync config from a directory.
 *
 * `{env:NAME}` tokens are resolved before parsing, so one checked-in
 * .raisindb-cli.yaml can target local, staging and prod:
 *
 *   server: "{env:RAISIN_SERVER:-http://localhost:8080}"
 *   branch: "{env:RAISIN_BRANCH:-main}"
 */
export function loadSyncConfig(
  directory: string,
  envOptions: EnvLoadOptions = {}
): SyncConfig | null {
  const configPath = findSyncConfig(directory);
  if (!configPath) {
    return null;
  }

  try {
    const raw = fs.readFileSync(configPath, 'utf-8');
    // Env files sit next to the sync config, not necessarily next to `directory`.
    const env = loadEnvContext(path.dirname(configPath), envOptions);
    const { text: content, unresolved } = substituteEnvTokens(raw, env);

    if (unresolved.length > 0) {
      const list = unresolved.map((t) => `${t.raw} (line ${t.line})`).join(', ');
      console.error(
        `Error loading sync config: unresolved token(s) in ${configPath}: ${list}. ` +
          'Set the variable in the environment or a .env file, or add an inline ' +
          'default {env:NAME:-fallback}'
      );
      return null;
    }

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
 * Save sync config to a directory.
 *
 * Refuses to overwrite a config that uses {env:...} tokens: the in-memory
 * config holds RESOLVED values, so writing it back would replace the tokens
 * with whatever this machine happened to have set.
 */
export function saveSyncConfig(directory: string, config: SyncConfig): void {
  const configPath = path.join(directory, SYNC_CONFIG_FILENAME);

  if (fs.existsSync(configPath)) {
    const existing = fs.readFileSync(configPath, 'utf-8');
    if (hasEnvTokens(existing)) {
      console.warn(
        `Not overwriting ${configPath}: it uses {env:...} tokens, and saving ` +
          'would replace them with resolved values. Edit the file directly.'
      );
      return;
    }
  }

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
