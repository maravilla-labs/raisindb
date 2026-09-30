import fs from 'fs';
import path from 'path';
import os from 'os';
import yaml from 'yaml';

export interface Config {
  server: string | null;
  token: string | null;
  default_repo: string | null;
}

const CONFIG_FILENAME = '.raisinrc';

/**
 * Searches up the directory tree for .raisinrc config file
 */
function findConfigFile(): string | null {
  let currentDir = process.cwd();
  const root = path.parse(currentDir).root;

  while (currentDir !== root) {
    const configPath = path.join(currentDir, CONFIG_FILENAME);
    if (fs.existsSync(configPath)) {
      return configPath;
    }
    currentDir = path.dirname(currentDir);
  }

  return null;
}

/**
 * Gets the config file path (searching up tree, then falling back to home directory)
 */
function getConfigPath(): string {
  // First try to find in current directory tree
  const foundConfig = findConfigFile();
  if (foundConfig) {
    return foundConfig;
  }

  // Fall back to home directory
  return path.join(os.homedir(), CONFIG_FILENAME);
}

/**
 * Loads the configuration from .raisinrc file
 */
export function loadConfig(): Config {
  const configPath = getConfigPath();

  if (!fs.existsSync(configPath)) {
    return {
      server: null,
      token: null,
      default_repo: null,
    };
  }

  try {
    const content = fs.readFileSync(configPath, 'utf-8');
    const config = yaml.parse(content);

    return {
      server: config.server || null,
      token: config.token || null,
      default_repo: config.default_repo || null,
    };
  } catch (error) {
    console.error(`Error reading config file: ${error instanceof Error ? error.message : String(error)}`);
    return {
      server: null,
      token: null,
      default_repo: null,
    };
  }
}

/**
 * Saves the configuration to .raisinrc file
 */
export function saveConfig(config: Config): void {
  const configPath = getConfigPath();

  try {
    const content = yaml.stringify({
      server: config.server,
      token: config.token,
      default_repo: config.default_repo,
    });

    fs.writeFileSync(configPath, content, 'utf-8');
  } catch (error) {
    console.error(`Error writing config file: ${error instanceof Error ? error.message : String(error)}`);
  }
}

/** Where the CLI talks to when nothing names a server. */
export const DEFAULT_SERVER = 'http://localhost:8081';

/**
 * The server named explicitly for this process — a command's `-s/--server`
 * flag. Set once at command start (the preAction hook in index.tsx) and by the
 * commands that take a server argument, so that EVERY HTTP call the command
 * makes goes to that server and never to the one in `.raisinrc`.
 */
let serverOverride: string | null = null;

/** Pin the server for the rest of this process. `null`/empty clears it. */
export function setServerOverride(server: string | null | undefined): void {
  serverOverride = typeof server === 'string' && server.trim() !== '' ? server.trim() : null;
}

/**
 * Pin a command's `-s/--server <url>` option, if it has one. Run from the
 * program's preAction hook for every command. A boolean `server` (the
 * `function test --server` switch with no URL) pins nothing.
 */
export function pinServerFromOptions(options: { server?: unknown } | undefined): void {
  const server = options?.server;
  if (typeof server === 'string' && server.trim() !== '') {
    setServerOverride(server);
  }
}

/** The pinned server, if any (see `setServerOverride`). */
export function getServerOverride(): string | null {
  return serverOverride;
}

/**
 * Gets the current server URL.
 *
 * Resolution order:
 *   1. an explicit `--server` for this command (`setServerOverride`)
 *   2. RAISINDB_SERVER environment variable
 *   3. .raisinrc config file (the server of the last `raisindb login`)
 */
export function getServer(): string | null {
  if (serverOverride) {
    return serverOverride;
  }
  const envServer = process.env.RAISINDB_SERVER;
  if (envServer && envServer.trim() !== '') {
    return envServer.trim();
  }
  const config = loadConfig();
  return config.server;
}

/** The server HTTP calls go to: `getServer()`, else the local default. */
export function getEffectiveServer(): string {
  return getServer() || DEFAULT_SERVER;
}

/**
 * Canonical form of a server URL, for comparing two of them: lower-case scheme
 * and host, default port dropped, trailing slashes dropped, `raisin://` and
 * `raisins://` read as `http://` and `https://`, a bare `host:port` read as
 * `http://`. Unparseable input comes back trimmed.
 */
export function normalizeServerUrl(server: string): string {
  let s = server.trim();
  if (s.startsWith('raisins://')) s = 'https://' + s.slice('raisins://'.length);
  else if (s.startsWith('raisin://')) s = 'http://' + s.slice('raisin://'.length);
  if (!/^[a-z][a-z0-9+.-]*:\/\//i.test(s)) s = 'http://' + s;
  try {
    const u = new URL(s);
    // URL lower-cases scheme and host and drops a default port on its own.
    return `${u.protocol}//${u.host}${u.pathname.replace(/\/+$/, '')}`;
  } catch {
    return server.trim().replace(/\/+$/, '');
  }
}

/** Target lines already printed, so a multi-request write announces once. */
const announcedTargets = new Set<string>();

/**
 * Say where a write is about to go — `→ http://localhost:8080 (repo studio)` —
 * before it happens, on stderr so `--json` output on stdout stays clean. Every
 * write command calls this: a deploy aimed at the wrong server must be visible
 * before it lands, not after. Printed once per distinct line per process.
 */
export function announceWriteTarget(detail?: string, server: string = getEffectiveServer()): void {
  const line = `→ ${server.replace(/\/+$/, '')}${detail ? ` (${detail})` : ''}`;
  if (announcedTargets.has(line)) return;
  announcedTargets.add(line);
  console.error(line);
}

/** Whether two server URLs name the same server (see `normalizeServerUrl`). */
export function sameServer(a: string | null | undefined, b: string | null | undefined): boolean {
  if (!a || !b) return false;
  return normalizeServerUrl(a) === normalizeServerUrl(b);
}

/**
 * Sets the server URL
 */
export function setServer(server: string): void {
  const config = loadConfig();
  config.server = server;
  saveConfig(config);
}

/**
 * Gets the default repository/database.
 *
 * Resolution order (env wins over config file, CI-friendly):
 *   1. RAISINDB_REPO environment variable
 *   2. .raisinrc config file
 */
export function getDefaultRepo(): string | null {
  const envRepo = process.env.RAISINDB_REPO;
  if (envRepo && envRepo.trim() !== '') {
    return envRepo.trim();
  }
  const config = loadConfig();
  return config.default_repo;
}

/**
 * Sets the default repository/database
 */
export function setDefaultRepo(repo: string): void {
  const config = loadConfig();
  config.default_repo = repo;
  saveConfig(config);
}
