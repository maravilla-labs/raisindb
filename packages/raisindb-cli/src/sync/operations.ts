/**
 * Sync operations - push and pull file changes
 */

import fs from 'fs';
import path from 'path';
import yaml from 'yaml';
import { SyncConfig } from './config.js';
import { getToken } from '../auth.js';
import { ChangeEvent } from './watcher.js';
import { decodeNamespace } from '../namespace-encoding.js';

/**
 * Extract a human-readable error message with cause chain
 */
function formatError(error: unknown): { message: string; details: string } {
  if (!(error instanceof Error)) {
    return { message: 'Unknown error', details: String(error) };
  }

  const message = error.message;
  const parts: string[] = [message];

  // Walk the cause chain (Node.js fetch wraps errors)
  let cause = (error as { cause?: unknown }).cause;
  while (cause) {
    if (cause instanceof Error) {
      parts.push(cause.message);
      cause = (cause as { cause?: unknown }).cause;
    } else {
      parts.push(String(cause));
      break;
    }
  }

  return {
    message: parts[0],
    details: parts.length > 1 ? parts.join(' → ') : message,
  };
}

/**
 * Result of a sync operation
 */
export interface SyncResult {
  success: boolean;
  path: string;
  operation: 'push' | 'pull';
  error?: string;
  /** Verbose error details (URL, cause chain, etc.) */
  details?: string;
  timestamp: number;
}

/**
 * Options for sync operations
 */
export interface SyncOperationOptions {
  /** Package directory */
  packageDir: string;
  /** Base directory for resolving file paths on disk.
   *  When a package has a content/ subdirectory, this is packageDir/content/
   *  so that logical paths (e.g. "functions/lib/...") map to the right files. */
  contentBase: string;
  /** Sync configuration */
  config: SyncConfig;
  /** Dry run mode */
  dryRun?: boolean;
  /** Force overwrite */
  force?: boolean;
}

/**
 * Parse a translation locale from a YAML filename.
 *
 * `.node.de.yaml` -> "de"
 * `.node.fr.yaml` -> "fr"
 * `about.de.yaml` -> "de"
 * `.node.yaml` -> null
 * `.node.index.js.yaml` -> null
 */
export function parseTranslationLocale(filename: string): string | null {
  if (!filename.endsWith('.yaml')) return null;

  const withoutYaml = filename.slice(0, -'.yaml'.length);

  if (filename.startsWith('.node.')) {
    const inner = withoutYaml.slice('.node.'.length);
    if (!inner) return null;
    // Valid BCP 47: 2-3 letter language, optional hyphen + 2-4 letter region
    if (/^[a-zA-Z]{2,3}(-[a-zA-Z]{2,4}|\d{3})?$/.test(inner)) {
      return inner;
    }
    return null;
  }

  // Named file: about.de.yaml
  const dotPos = withoutYaml.lastIndexOf('.');
  if (dotPos < 0) return null;
  const candidate = withoutYaml.slice(dotPos + 1);
  if (!candidate) return null;
  if (/^[a-zA-Z]{2,3}(-[a-zA-Z]{2,4}|\d{3})?$/.test(candidate)) {
    return candidate;
  }
  return null;
}

/**
 * Keys that should be skipped when converting translations to JSON Pointers.
 * These are structural keys, not translatable content.
 * Must match NON_TRANSLATABLE_KEYS in the server's package_install/translation.rs.
 */
const NON_TRANSLATABLE_KEYS = new Set([
  'uuid', 'id', 'element_type', 'slug', 'node_type', 'archetype',
  'parent', 'order', 'sort_order', 'weight',
]);

/**
 * Check if an array is a UUID-keyed section (array of objects with `uuid` fields).
 */
function isUuidSection(arr: unknown[]): boolean {
  return arr.length > 0 && arr.every(
    (item) => typeof item === 'object' && item !== null && 'uuid' in item
  );
}

/**
 * Convert translation YAML (flat key/value with optional UUID-keyed sections)
 * to JSON Pointer format expected by the server's translate command.
 *
 * Simple keys:    { title: "Hola" }         → { "/title": "Hola" }
 * Section arrays: { content: [{ uuid: "hero-1", headline: "Vision" }] }
 *                 → { "/content/hero-1/headline": "Vision" }
 */
function translationsToPointers(
  obj: Record<string, unknown>
): Record<string, unknown> {
  const result: Record<string, unknown> = {};

  for (const [key, val] of Object.entries(obj)) {
    if (NON_TRANSLATABLE_KEYS.has(key)) continue;

    if (Array.isArray(val) && isUuidSection(val)) {
      for (const item of val) {
        const record = item as Record<string, unknown>;
        const uuid = record.uuid as string;
        for (const [field, fieldVal] of Object.entries(record)) {
          if (NON_TRANSLATABLE_KEYS.has(field)) continue;
          result[`/${key}/${uuid}/${field}`] = fieldVal;
        }
      }
    } else {
      result[`/${key}`] = val;
    }
  }

  return result;
}

/**
 * Derive the base node file path from a translation file path.
 * This lets us reuse buildServerUrl to get the correct server URL.
 *
 * "launchpad/launchpad/home/.node.de.yaml" → "launchpad/launchpad/home/.node.yaml"
 * "launchpad/launchpad/about.de.yaml"      → "launchpad/launchpad/about.yaml"
 */
function deriveBaseNodeFilePath(translationFilePath: string): string {
  const dir = path.dirname(translationFilePath);
  const filename = path.basename(translationFilePath);

  let baseFilename: string;
  if (filename.startsWith('.node.')) {
    baseFilename = '.node.yaml';
  } else {
    const withoutYaml = filename.slice(0, -'.yaml'.length);
    const dotPos = withoutYaml.lastIndexOf('.');
    baseFilename = withoutYaml.slice(0, dotPos) + '.yaml';
  }

  return dir ? `${dir}/${baseFilename}` : baseFilename;
}

/**
 * Push a translation file to the server
 */
export async function pushTranslationFile(
  filePath: string,
  options: SyncOperationOptions
): Promise<SyncResult> {
  const { contentBase, config, dryRun } = options;
  const fullPath = path.join(contentBase, filePath);
  const timestamp = Date.now();

  try {
    if (!fs.existsSync(fullPath)) {
      return {
        success: false,
        path: filePath,
        operation: 'push',
        error: 'File not found',
        timestamp,
      };
    }

    const filename = path.basename(filePath);
    const locale = parseTranslationLocale(filename);
    if (!locale) {
      return {
        success: false,
        path: filePath,
        operation: 'push',
        error: 'Not a translation file',
        timestamp,
      };
    }

    if (dryRun) {
      return {
        success: true,
        path: filePath,
        operation: 'push',
        timestamp,
      };
    }

    const content = fs.readFileSync(fullPath, 'utf-8');
    const rawTranslations = yaml.parse(content) || {};

    const token = getToken();
    if (!token) {
      return {
        success: false,
        path: filePath,
        operation: 'push',
        error: 'Not authenticated',
        timestamp,
      };
    }

    // Derive the base node path and reuse buildServerUrl for correct URL
    const baseFilePath = deriveBaseNodeFilePath(filePath);
    const { url: nodeUrl } = buildServerUrl(config, baseFilePath);

    // Convert flat YAML translations to JSON Pointer format
    const translations = translationsToPointers(rawTranslations);

    // POST to the translate command endpoint
    const url = `${nodeUrl}/raisin:cmd/translate`;
    const response = await fetch(url, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        Authorization: `Bearer ${token}`,
      },
      body: JSON.stringify({
        locale,
        translations,
        message: 'Package sync',
        actor: 'cli',
      }),
    });

    if (!response.ok) {
      const errorText = await response.text();
      return {
        success: false,
        path: filePath,
        operation: 'push',
        error: `${response.status} ${response.statusText}`,
        details: `POST ${url}\n${response.status} ${response.statusText}: ${errorText}`,
        timestamp,
      };
    }

    return {
      success: true,
      path: filePath,
      operation: 'push',
      timestamp,
    };
  } catch (error) {
    const { message, details } = formatError(error);
    return {
      success: false,
      path: filePath,
      operation: 'push',
      error: message,
      details,
      timestamp,
    };
  }
}

/**
 * Build the server URL for a given file path.
 *
 * File paths from getLocalFiles() look like:
 *   "functions/lib/raisin/ai/agent-handler/.node.yaml"
 *   "functions/lib/raisin/ai/agent-handler/index.js"
 *   "nodetypes/ai_agent.yaml"
 *
 * The first segment is the workspace, the rest is the node path.
 * For .node.yaml files the node path is the containing directory.
 */
/**
 * Convert raisin:// or raisins:// URLs to http:// or https://
 */
function toHttpUrl(server: string): string {
  if (server.startsWith('raisins://')) {
    return server.replace('raisins://', 'https://');
  }
  if (server.startsWith('raisin://')) {
    return server.replace('raisin://', 'http://');
  }
  return server;
}

function buildServerUrl(
  config: SyncConfig,
  filePath: string
): { url: string; workspace: string; nodePath: string } {
  const parts = filePath.split('/');
  const workspace = decodeNamespace(parts[0]);
  const rest = parts.slice(1); // e.g. ["lib", "raisin", "ai", "agent-handler", ".node.yaml"]

  const filename = path.basename(filePath);
  let nodePath: string;

  if (filename === '.node.yaml') {
    // .node.yaml describes the directory node itself
    nodePath = rest.slice(0, -1).join('/'); // drop ".node.yaml"
  } else {
    nodePath = rest.join('/');
  }

  const httpServer = toHttpUrl(config.server);
  const url = `${httpServer}/api/repository/${config.repository}/${config.branch}/head/${workspace}/${nodePath}`;
  return { url, workspace, nodePath };
}

/**
 * MIME types for code files uploaded as binary assets.
 */
const CODE_MIME_TYPES: Record<string, string> = {
  '.js': 'text/javascript',
  '.py': 'text/x-python',
  '.star': 'text/x-starlark',
};

/**
 * Push a code file (.js, .py, .star) to the server via multipart upload.
 *
 * Code files are raisin:Asset nodes that require a `file` property.
 * The server expects a multipart POST with a `file` field — it streams
 * the binary to storage and creates a Resource property with storage_key.
 */
async function pushCodeFile(
  filePath: string,
  options: SyncOperationOptions
): Promise<SyncResult> {
  const { contentBase, config } = options;
  const fullPath = path.join(contentBase, filePath);
  const timestamp = Date.now();
  const { url } = buildServerUrl(config, filePath);
  const uploadUrl = `${url}?override_existing=true`;

  try {
    const token = getToken();
    if (!token) {
      return {
        success: false,
        path: filePath,
        operation: 'push',
        error: 'Not authenticated',
        timestamp,
      };
    }

    const buffer = fs.readFileSync(fullPath);
    const filename = path.basename(filePath);
    const ext = path.extname(filePath);
    const mimeType = CODE_MIME_TYPES[ext] || 'application/octet-stream';

    const formData = new FormData();
    formData.append('file', new Blob([buffer], { type: mimeType }), filename);

    const response = await fetch(uploadUrl, {
      method: 'POST',
      headers: {
        Authorization: `Bearer ${token}`,
      },
      body: formData,
    });

    if (!response.ok) {
      const errorText = await response.text();
      return {
        success: false,
        path: filePath,
        operation: 'push',
        error: `${response.status} ${response.statusText}`,
        details: `POST ${uploadUrl}\n${response.status} ${response.statusText}: ${errorText}`,
        timestamp,
      };
    }

    return {
      success: true,
      path: filePath,
      operation: 'push',
      timestamp,
    };
  } catch (error) {
    const { message, details } = formatError(error);
    return {
      success: false,
      path: filePath,
      operation: 'push',
      error: message,
      details: `POST ${uploadUrl}\n${details}`,
      timestamp,
    };
  }
}

/**
 * Create a node on the server from a .node.yaml file (POST fallback when PUT returns 404).
 *
 * The POST endpoint expects a full node body with name, node_type, and properties.
 * It creates the node as a child of the parent path.
 */
async function createNodeFromYaml(
  filePath: string,
  content: string,
  options: SyncOperationOptions,
  token: string,
  timestamp: number
): Promise<SyncResult> {
  const { config, force } = options;
  const { url, nodePath } = buildServerUrl(config, filePath);

  // Derive parent URL and node name from the node path
  const lastSlash = url.lastIndexOf('/');
  const parentUrl = url.slice(0, lastSlash);
  const nodeName = nodePath.split('/').pop() || 'node';

  const parsed = yaml.parse(content) || {};
  const nodeType = parsed.node_type || 'raisin:Node';
  let properties: Record<string, unknown>;
  if (parsed.properties) {
    properties = parsed.properties;
  } else {
    const { node_type: _, ...rest } = parsed;
    properties = rest;
  }

  const postBody = {
    name: nodeName,
    node_type: nodeType,
    path: `/${nodePath}`,
    properties,
  };

  try {
    const response = await fetch(parentUrl, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        Authorization: `Bearer ${token}`,
      },
      body: JSON.stringify(postBody),
    });

    if (!response.ok) {
      // If POST fails with 400 (node already exists) and force is enabled,
      // retry with PUT to the original node URL to update properties
      if (response.status === 400 && force) {
        const putBody = buildPushBody(filePath, content);
        const putResponse = await fetch(url, {
          method: 'PUT',
          headers: {
            'Content-Type': 'application/json',
            Authorization: `Bearer ${token}`,
          },
          body: JSON.stringify(putBody),
        });

        if (!putResponse.ok) {
          const putErrorText = await putResponse.text();
          return {
            success: false,
            path: filePath,
            operation: 'push',
            error: `${putResponse.status} ${putResponse.statusText}`,
            details: `PUT ${url} (force retry)\n${putResponse.status} ${putResponse.statusText}: ${putErrorText}`,
            timestamp,
          };
        }

        return {
          success: true,
          path: filePath,
          operation: 'push',
          timestamp,
        };
      }

      const errorText = await response.text();
      return {
        success: false,
        path: filePath,
        operation: 'push',
        error: `${response.status} ${response.statusText}`,
        details: `POST ${parentUrl} (create)\n${response.status} ${response.statusText}: ${errorText}`,
        timestamp,
      };
    }

    return {
      success: true,
      path: filePath,
      operation: 'push',
      timestamp,
    };
  } catch (error) {
    const { message, details } = formatError(error);
    return {
      success: false,
      path: filePath,
      operation: 'push',
      error: message,
      details: `POST ${parentUrl} (create)\n${details}`,
      timestamp,
    };
  }
}

/**
 * Build the request body for a push, depending on the file type.
 */
function buildPushBody(
  filePath: string,
  content: string
): Record<string, unknown> {
  const filename = path.basename(filePath);
  const ext = path.extname(filePath);

  if (filename === '.node.yaml') {
    // .node.yaml: parse YAML and send properties (and node_type if present)
    const parsed = yaml.parse(content) || {};
    const body: Record<string, unknown> = {};
    if (parsed.properties) {
      body.properties = parsed.properties;
    } else {
      // The whole file is properties (minus node_type)
      const { node_type, ...rest } = parsed;
      body.properties = rest;
    }
    return body;
  }

  if (['.yaml', '.yml'].includes(ext)) {
    // Other YAML files: parse and send as properties
    const parsed = yaml.parse(content) || {};
    if (parsed.properties) {
      return { properties: parsed.properties };
    }
    return { properties: parsed };
  }

  if (ext === '.json') {
    const parsed = JSON.parse(content);
    return { properties: parsed };
  }

  // Fallback: send raw content
  return { properties: { content } };
}

/**
 * Push a local file to the server
 */
export async function pushFile(
  filePath: string,
  options: SyncOperationOptions
): Promise<SyncResult> {
  const { contentBase, config, dryRun } = options;
  const fullPath = path.join(contentBase, filePath);
  const timestamp = Date.now();

  try {
    // Check if file exists
    if (!fs.existsSync(fullPath)) {
      return {
        success: false,
        path: filePath,
        operation: 'push',
        error: 'File not found',
        timestamp,
      };
    }

    // Detect translation files and delegate
    const filename = path.basename(filePath);
    if (parseTranslationLocale(filename)) {
      return pushTranslationFile(filePath, options);
    }

    if (dryRun) {
      return {
        success: true,
        path: filePath,
        operation: 'push',
        timestamp,
      };
    }

    // Code files (.js, .py, .star) need multipart upload
    const ext = path.extname(filePath);
    if (['.js', '.py', '.star'].includes(ext)) {
      return pushCodeFile(filePath, options);
    }

    // Read file content
    const content = fs.readFileSync(fullPath, 'utf-8');

    // Get authentication token
    const token = getToken();
    if (!token) {
      return {
        success: false,
        path: filePath,
        operation: 'push',
        error: 'Not authenticated',
        timestamp,
      };
    }

    // Build URL and body
    const { url } = buildServerUrl(config, filePath);
    const body = buildPushBody(filePath, content);

    // Push to server via HTTP API
    const response = await fetch(url, {
      method: 'PUT',
      headers: {
        'Content-Type': 'application/json',
        Authorization: `Bearer ${token}`,
      },
      body: JSON.stringify(body),
    });

    if (!response.ok) {
      // If node doesn't exist and this is a .node.yaml, create it via POST
      if (response.status === 404 && filename === '.node.yaml') {
        return createNodeFromYaml(filePath, content, options, token, timestamp);
      }

      const errorText = await response.text();
      return {
        success: false,
        path: filePath,
        operation: 'push',
        error: `${response.status} ${response.statusText}`,
        details: `PUT ${url}\n${response.status} ${response.statusText}: ${errorText}`,
        timestamp,
      };
    }

    return {
      success: true,
      path: filePath,
      operation: 'push',
      timestamp,
    };
  } catch (error) {
    const { url } = buildServerUrl(config, filePath);
    const { message, details } = formatError(error);
    return {
      success: false,
      path: filePath,
      operation: 'push',
      error: message,
      details: `PUT ${url}\n${details}`,
      timestamp,
    };
  }
}

/**
 * Pull a file from the server to local
 */
export async function pullFile(
  filePath: string,
  options: SyncOperationOptions
): Promise<SyncResult> {
  const { contentBase, config, dryRun, force } = options;
  const fullPath = path.join(contentBase, filePath);
  const timestamp = Date.now();

  try {
    // Check if local file exists and we're not forcing
    if (fs.existsSync(fullPath) && !force && !dryRun) {
      // Could check modification time here for better conflict detection
    }

    const token = getToken();
    if (!token) {
      return {
        success: false,
        path: filePath,
        operation: 'pull',
        error: 'Not authenticated',
        timestamp,
      };
    }

    // Fetch from server
    const { url } = buildServerUrl(config, filePath);
    const response = await fetch(url, {
      headers: {
        Authorization: `Bearer ${token}`,
      },
    });

    if (!response.ok) {
      if (response.status === 404) {
        return {
          success: false,
          path: filePath,
          operation: 'pull',
          error: 'File not found on server',
          timestamp,
        };
      }
      const errorText = await response.text();
      return {
        success: false,
        path: filePath,
        operation: 'pull',
        error: `Server error: ${response.status} - ${errorText}`,
        timestamp,
      };
    }

    if (dryRun) {
      return {
        success: true,
        path: filePath,
        operation: 'pull',
        timestamp,
      };
    }

    // Parse response and write to file
    const data = await response.json() as Record<string, unknown>;
    const ext = path.extname(filePath);

    // Ensure directory exists
    const dir = path.dirname(fullPath);
    if (!fs.existsSync(dir)) {
      fs.mkdirSync(dir, { recursive: true });
    }

    // Write content based on type
    const properties = data.properties as Record<string, unknown> | undefined;
    if (['.yaml', '.yml'].includes(ext)) {
      const content = yaml.stringify(properties || data);
      fs.writeFileSync(fullPath, content, 'utf-8');
    } else if (ext === '.json') {
      fs.writeFileSync(fullPath, JSON.stringify(properties || data, null, 2), 'utf-8');
    } else {
      const content = (properties?.content || (data as Record<string, unknown>).content || '') as string;
      fs.writeFileSync(fullPath, content, 'utf-8');
    }

    return {
      success: true,
      path: filePath,
      operation: 'pull',
      timestamp,
    };
  } catch (error) {
    return {
      success: false,
      path: filePath,
      operation: 'pull',
      error: error instanceof Error ? error.message : 'Unknown error',
      timestamp,
    };
  }
}

/**
 * Delete a file from the server
 */
export async function deleteFromServer(
  filePath: string,
  options: SyncOperationOptions
): Promise<SyncResult> {
  const { config, dryRun } = options;
  const timestamp = Date.now();

  try {
    if (dryRun) {
      return {
        success: true,
        path: filePath,
        operation: 'push',
        timestamp,
      };
    }

    const token = getToken();
    if (!token) {
      return {
        success: false,
        path: filePath,
        operation: 'push',
        error: 'Not authenticated',
        timestamp,
      };
    }

    const { url } = buildServerUrl(config, filePath);
    const response = await fetch(url, {
      method: 'DELETE',
      headers: {
        Authorization: `Bearer ${token}`,
      },
    });

    if (!response.ok && response.status !== 404) {
      const errorText = await response.text();
      return {
        success: false,
        path: filePath,
        operation: 'push',
        error: `Server error: ${response.status} - ${errorText}`,
        timestamp,
      };
    }

    return {
      success: true,
      path: filePath,
      operation: 'push',
      timestamp,
    };
  } catch (error) {
    return {
      success: false,
      path: filePath,
      operation: 'push',
      error: error instanceof Error ? error.message : 'Unknown error',
      timestamp,
    };
  }
}

/**
 * Delete a local file (from server change)
 */
export async function deleteLocal(
  filePath: string,
  options: SyncOperationOptions
): Promise<SyncResult> {
  const { contentBase, dryRun } = options;
  const fullPath = path.join(contentBase, filePath);
  const timestamp = Date.now();

  try {
    if (!fs.existsSync(fullPath)) {
      return {
        success: true,
        path: filePath,
        operation: 'pull',
        timestamp,
      };
    }

    if (dryRun) {
      return {
        success: true,
        path: filePath,
        operation: 'pull',
        timestamp,
      };
    }

    fs.unlinkSync(fullPath);

    return {
      success: true,
      path: filePath,
      operation: 'pull',
      timestamp,
    };
  } catch (error) {
    return {
      success: false,
      path: filePath,
      operation: 'pull',
      error: error instanceof Error ? error.message : 'Unknown error',
      timestamp,
    };
  }
}

/**
 * Process a batch of local changes
 */
export async function processLocalChanges(
  changes: ChangeEvent[],
  options: SyncOperationOptions
): Promise<SyncResult[]> {
  const results: SyncResult[] = [];

  for (const change of changes) {
    let result: SyncResult;

    if (change.type === 'unlink' || change.type === 'unlinkDir') {
      result = await deleteFromServer(change.path, options);
    } else {
      result = await pushFile(change.path, options);
    }

    results.push(result);
  }

  return results;
}

/**
 * Process a batch of server changes
 */
export async function processServerChanges(
  changes: ChangeEvent[],
  options: SyncOperationOptions
): Promise<SyncResult[]> {
  const results: SyncResult[] = [];

  for (const change of changes) {
    let result: SyncResult;

    if (change.type === 'unlink' || change.type === 'unlinkDir') {
      result = await deleteLocal(change.path, options);
    } else {
      result = await pullFile(change.path, options);
    }

    results.push(result);
  }

  return results;
}

/**
 * Resolve a conflict between local and server changes
 */
export type ConflictStrategy = 'local-wins' | 'server-wins' | 'newest-wins' | 'skip';

export interface Conflict {
  local: ChangeEvent;
  server: ChangeEvent;
}

export async function resolveConflict(
  conflict: Conflict,
  strategy: ConflictStrategy,
  options: SyncOperationOptions
): Promise<SyncResult | null> {
  const { local, server } = conflict;

  switch (strategy) {
    case 'local-wins':
      // Push local version to server
      if (local.type === 'unlink') {
        return deleteFromServer(local.path, options);
      }
      return pushFile(local.path, options);

    case 'server-wins':
      // Pull server version to local
      if (server.type === 'unlink') {
        return deleteLocal(server.path, options);
      }
      return pullFile(server.path, options);

    case 'newest-wins':
      // Compare timestamps and use newer version
      if (local.timestamp > server.timestamp) {
        return resolveConflict(conflict, 'local-wins', options);
      }
      return resolveConflict(conflict, 'server-wins', options);

    case 'skip':
      // Don't sync this file
      return null;

    default:
      return null;
  }
}
