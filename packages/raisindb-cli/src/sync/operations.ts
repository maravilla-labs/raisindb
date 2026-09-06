/**
 * Sync operations - push and pull file changes
 */

import fs from 'fs';
import path from 'path';
import yaml from 'yaml';
import { SyncConfig } from './config.js';
import { getToken } from '../auth.js';
import { ChangeEvent } from './watcher.js';
import { mapChangeToNode, parseTranslationLocale, type SchemaKind } from './mapping.js';
import {
  EnvContext,
  emptyEnvContext,
  hasEnvTokens,
  substituteEnvTokens,
} from '../env/substitute.js';
import { isSubstitutableFile } from '../env/load.js';

export { parseTranslationLocale } from './mapping.js';

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
  /** Environment for {env:...} substitution. Defaults to inline defaults only. */
  env?: EnvContext;
}

/**
 * Read a text file, resolving `{env:NAME}` tokens.
 *
 * Every push path reads through this — a path that read the file directly would
 * upload a literal `{env:PREVIEW_SERVER}` while its neighbours upload resolved
 * values, and the difference would only show up at runtime.
 *
 * Unresolved tokens are reported as an error string rather than thrown: sync is
 * per-file, so one bad file should fail on its own without aborting the run.
 */
function readSubstituted(
  fullPath: string,
  relPath: string,
  env: EnvContext = emptyEnvContext()
): { text: string; error?: string } {
  const raw = fs.readFileSync(fullPath, 'utf-8');

  if (!isSubstitutableFile(relPath) || !hasEnvTokens(raw)) {
    return { text: raw };
  }

  const { text, unresolved } = substituteEnvTokens(raw, env);
  if (unresolved.length > 0) {
    const list = unresolved
      .map((t) => `${t.raw} (line ${t.line})`)
      .join(', ');
    return {
      text,
      error:
        `Unresolved {env:...} token(s): ${list}. Set the variable in the ` +
        'environment or a .env file, or add an inline default {env:NAME:-fallback}',
    };
  }

  return { text };
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

    const { text: content, error: envError } = readSubstituted(
      fullPath,
      filePath,
      options.env
    );
    if (envError) {
      return { success: false, path: filePath, operation: 'push', error: envError, timestamp };
    }
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
 *
 * Exported so the wasm function dev loop (`wasm-fn/run-client.ts`) resolves a
 * server URL exactly as `sync` does, rather than growing a second copy.
 */
export function toHttpUrl(server: string): string {
  // A missing `server` used to reach here as `undefined` and throw
  // `Cannot read properties of undefined (reading 'startsWith')` — the symptom
  // of a package install policy being loaded as a CLI config. `config.ts` now
  // rejects that file by shape, so this is the second line of defence: a
  // malformed config should say what is wrong, not fail inside a URL helper.
  if (typeof server !== 'string' || !server) {
    throw new Error(
      'No `server` in the sync config. Add one, or remove the config file to fall back to ~/.raisinrc.'
    );
  }
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
  filePath: string,
  explicitName?: string
): { url: string; workspace: string; nodePath: string } {
  const mapped = mapChangeToNode(filePath, explicitName);
  const workspace = mapped.workspace || filePath.split('/')[0];
  const nodePath = mapped.nodePath ?? filePath.split('/').slice(1).join('/');

  const httpServer = toHttpUrl(config.server);
  const url = `${httpServer}/api/repository/${config.repository}/${config.branch}/head/${workspace}/${nodePath}`;
  return { url, workspace, nodePath };
}

/** Management endpoint (plural) + request-body field for each schema kind. */
const SCHEMA_ENDPOINTS: Record<SchemaKind, { plural: string; field: string }> = {
  nodetype: { plural: 'nodetypes', field: 'node_type' },
  archetype: { plural: 'archetypes', field: 'archetype' },
  elementtype: { plural: 'elementtypes', field: 'element_type' },
  mixin: { plural: 'mixins', field: 'node_type' },
};

/**
 * Push a schema definition (node type / archetype / element type / mixin) to
 * the management API.
 *
 * POST to the create endpoint upserts (create-or-update) server-side, so this
 * applies changed definitions live — no re-deploy needed. The node name comes
 * from the YAML's `name` field, so the server addresses the right type.
 */
async function pushSchemaFile(
  filePath: string,
  options: SyncOperationOptions,
  schemaKind: SchemaKind
): Promise<SyncResult> {
  const { packageDir, config, dryRun } = options;
  // Schema directories live at the package root (a sibling of content/), so
  // resolve against packageDir, not the content base.
  const fullPath = path.join(packageDir, filePath);
  const timestamp = Date.now();
  const { plural, field } = SCHEMA_ENDPOINTS[schemaKind];
  const httpServer = toHttpUrl(config.server);
  const url = `${httpServer}/api/management/${config.repository}/${config.branch}/${plural}`;

  try {
    if (dryRun) {
      return { success: true, path: filePath, operation: 'push', timestamp };
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

    const { text: content, error: envError } = readSubstituted(
      fullPath,
      filePath,
      options.env
    );
    if (envError) {
      return { success: false, path: filePath, operation: 'push', error: envError, timestamp };
    }
    const definition = yaml.parse(content);
    if (!definition || typeof definition !== 'object') {
      return {
        success: false,
        path: filePath,
        operation: 'push',
        error: 'Invalid schema YAML (expected an object)',
        timestamp,
      };
    }

    const response = await fetch(url, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        Authorization: `Bearer ${token}`,
      },
      body: JSON.stringify({
        [field]: definition,
        commit: { message: 'Package sync', actor: 'cli' },
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

    return { success: true, path: filePath, operation: 'push', timestamp };
  } catch (error) {
    const { message, details } = formatError(error);
    return {
      success: false,
      path: filePath,
      operation: 'push',
      error: message,
      details: `POST ${url}\n${details}`,
      timestamp,
    };
  }
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
 * Push a code file (.js, .py, .star) as the inline `code` property of its
 * raisin:Asset node — the same shape examples/shiftboard/setup.mjs uses.
 *
 * The function runtime prefers the inline `code` property over the stored
 * binary, so this makes edits live immediately. If the asset node does not
 * exist yet, it is created under its parent (function) node.
 */
async function pushCodeInline(
  filePath: string,
  options: SyncOperationOptions
): Promise<SyncResult> {
  const { contentBase, config } = options;
  const fullPath = path.join(contentBase, filePath);
  const timestamp = Date.now();
  const { url, nodePath } = buildServerUrl(config, filePath);

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

    const { text: code, error: envError } = readSubstituted(
      fullPath,
      filePath,
      options.env
    );
    if (envError) {
      return { success: false, path: filePath, operation: 'push', error: envError, timestamp };
    }
    const headers = {
      'Content-Type': 'application/json',
      Authorization: `Bearer ${token}`,
    };

    // Read-modify-write: PUT replaces all properties, so fetch the existing
    // ones first to preserve the required `file` resource etc.
    let response = await fetch(url, { headers });

    if (response.ok) {
      const node = (await response.json()) as {
        properties?: Record<string, unknown>;
      };
      const properties = { ...(node.properties || {}), code };
      response = await fetch(url, {
        method: 'PUT',
        headers,
        body: JSON.stringify({ properties }),
      });
    }

    if (response.status === 404) {
      // Asset node missing — create it under the parent (function) node,
      // mirroring setup.mjs ensureNode.
      const lastSlash = url.lastIndexOf('/');
      const parentUrl = url.slice(0, lastSlash);
      const name = nodePath.split('/').pop() || path.basename(filePath);
      response = await fetch(parentUrl, {
        method: 'POST',
        headers,
        body: JSON.stringify({
          node: {
            name,
            node_type: 'raisin:Asset',
            properties: { title: name, file: '', code },
          },
        }),
      });
    }

    if (!response.ok) {
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

    return { success: true, path: filePath, operation: 'push', timestamp };
  } catch (error) {
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
 * Push a binary/asset file to the server via multipart upload.
 *
 * Asset files are raisin:Asset nodes that require a `file` property.
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
  timestamp: number,
  explicitName?: string
): Promise<SyncResult> {
  const { config, force } = options;
  // Must use the SAME name the caller used to build the PUT url, or the node
  // is created at a path the next push will 404 on again.
  const { url, nodePath } = buildServerUrl(config, filePath, explicitName);

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
    const { node_type: _, archetype: __, ...rest } = parsed;
    properties = rest;
  }

  const postBody: Record<string, unknown> = {
    name: nodeName,
    node_type: nodeType,
    path: `/${nodePath}`,
    properties,
  };
  if (parsed.archetype) {
    postBody.archetype = parsed.archetype;
  }

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
 * Properties of a `raisin:Asset` node that are derived from the BINARY, not
 * authored. The installer recomputes all of these from the stored blob every
 * time (`install_binary_file` in
 * crates/raisin-rocksdb/.../install_content/node_installer.rs), so a metadata
 * file must never push them: `file` in particular is the Resource pointing at
 * the blob, and overwriting it with an authored value unbinds the asset from
 * its bytes.
 */
const ASSET_SERVER_OWNED_PROPS = [
  'file',
  'file_type',
  'file_size',
  'content_hash',
];

/**
 * Push a `.node.{filename}.yaml` asset-metadata file.
 *
 * Applies the same fields the installer's `AssetFileDef` accepts -- `title`
 * (defaulting to the filename), `description`, and any extra `properties` --
 * onto the existing asset node, leaving the binary-derived properties alone.
 *
 * This used to be classified as `skip` ("applied at install time"), so editing
 * an asset's title did nothing until the next `deploy --install`.
 */
async function pushAssetMetadata(
  filePath: string,
  options: SyncOperationOptions,
  nodePath: string
): Promise<SyncResult> {
  const { contentBase, config } = options;
  const timestamp = Date.now();
  const fullPath = path.join(contentBase, filePath);

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

    const { text: content, error: envError } = readSubstituted(
      fullPath,
      filePath,
      options.env
    );
    if (envError) {
      return { success: false, path: filePath, operation: 'push', error: envError, timestamp };
    }

    const parsed = yaml.parse(content) || {};
    const targetFilename = nodePath.split('/').pop() || '';

    // Mirror AssetFileDef: title defaults to the filename, description is
    // optional, and `properties` carries anything else.
    const authored: Record<string, unknown> = { ...(parsed.properties || {}) };
    authored.title =
      typeof parsed.title === 'string' ? parsed.title : targetFilename;
    if (typeof parsed.description === 'string') {
      authored.description = parsed.description;
    }
    for (const owned of ASSET_SERVER_OWNED_PROPS) {
      delete authored[owned];
    }

    // The mapping already resolved this to the SIBLING asset's node path, so
    // buildServerUrl points at the asset node, not at the metadata file.
    const { url } = buildServerUrl(config, filePath);
    const headers = {
      'Content-Type': 'application/json',
      Authorization: `Bearer ${token}`,
    };

    // Read-modify-write: PUT replaces ALL properties, so the binary-derived
    // ones have to be read back and re-sent or the asset loses its bytes.
    const existing = await fetch(url, { headers });
    if (existing.status === 404) {
      return {
        success: false,
        path: filePath,
        operation: 'push',
        error: `No asset node at '${nodePath}' yet`,
        details:
          `Asset metadata describes a sibling file. Push '${targetFilename}' ` +
          'first (or re-save this file afterwards) so the asset node exists.',
        timestamp,
      };
    }
    if (!existing.ok) {
      const errorText = await existing.text();
      return {
        success: false,
        path: filePath,
        operation: 'push',
        error: `${existing.status} ${existing.statusText}`,
        details: `GET ${url}\n${existing.status} ${existing.statusText}: ${errorText}`,
        timestamp,
      };
    }

    const node = (await existing.json()) as {
      properties?: Record<string, unknown>;
    };
    const properties = { ...(node.properties || {}), ...authored };

    const response = await fetch(url, {
      method: 'PUT',
      headers,
      body: JSON.stringify({ properties }),
    });

    if (!response.ok) {
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

    return { success: true, path: filePath, operation: 'push', timestamp };
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
 * Build the request body for a push, depending on the file type.
 */
function buildPushBody(
  filePath: string,
  content: string
): Record<string, unknown> {
  const ext = path.extname(filePath);

  if (['.yaml', '.yml'].includes(ext)) {
    // Every node YAML, `.node.yaml` and flat `{name}.yaml` alike, is read the
    // same way. The flat form used to skip the structural-key strip and push
    // `node_type` in as an ordinary property.
    //
    // On `name`, see `nodeNameOverride` below: it is a node-name override ONLY
    // when the file separates structure from content with a `properties:`
    // block. In a flat file it is just another property, which is what a
    // required `name` field (e.g. `raisin:Role`) needs it to be.
    const parsed = yaml.parse(content) || {};
    const body: Record<string, unknown> = {};
    if (parsed.properties) {
      body.properties = parsed.properties;
    } else {
      const { node_type: _nt, archetype: _at, ...rest } = parsed;
      body.properties = rest;
    }
    if (parsed.archetype) {
      body.archetype = parsed.archetype;
    }
    return body;
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
  const timestamp = Date.now();

  try {
    // Classify the change (pure mapping, see sync/mapping.ts). Schema/skip/
    // structural kinds are resolved before the content-base existence check
    // because they don't live under contentBase (schema dirs sit at the
    // package root; skip/structural don't read a content file at all).
    const mapped = mapChangeToNode(filePath);

    if (mapped.kind === 'skip') {
      // Nothing to push (hidden/metadata file) — report success, no-op
      return { success: true, path: filePath, operation: 'push', timestamp };
    }

    // Schema definitions (node types / archetypes / element types / mixins)
    // resolve against packageDir and are pushed to the management API (upsert).
    if (mapped.kind === 'schema' && mapped.schemaKind) {
      return pushSchemaFile(filePath, options, mapped.schemaKind);
    }

    if (mapped.kind === 'structural') {
      return {
        success: false,
        path: filePath,
        operation: 'push',
        error: mapped.reason || 'structural change — run deploy --install',
        timestamp,
      };
    }

    // Remaining kinds are content under the content base — it must exist.
    const fullPath = path.join(contentBase, filePath);
    if (!fs.existsSync(fullPath)) {
      return {
        success: false,
        path: filePath,
        operation: 'push',
        error: 'File not found',
        timestamp,
      };
    }

    if (mapped.kind === 'translation') {
      return pushTranslationFile(filePath, options);
    }

    if (mapped.kind === 'asset-metadata' && mapped.nodePath) {
      if (dryRun) {
        return { success: true, path: filePath, operation: 'push', timestamp };
      }
      return pushAssetMetadata(filePath, options, mapped.nodePath);
    }

    if (dryRun) {
      return {
        success: true,
        path: filePath,
        operation: 'push',
        timestamp,
      };
    }

    // Function code files (.js, .py, .star) → inline `code` property
    if (mapped.kind === 'code') {
      return pushCodeInline(filePath, options);
    }

    // Other binary assets → multipart upload
    if (mapped.kind === 'asset') {
      return pushCodeFile(filePath, options);
    }

    // Read file content, resolving {env:...} tokens
    const { text: content, error: envError } = readSubstituted(
      fullPath,
      filePath,
      options.env
    );
    if (envError) {
      return {
        success: false,
        path: filePath,
        operation: 'push',
        error: envError,
        timestamp,
      };
    }

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

    // Build URL and body.
    //
    // `properties.name` is deliberately NOT consulted as a name override: it is
    // content (a role's display name, a page's title), and letting it decide
    // the node's location is what made the installer and this pusher disagree
    // about where a file belongs. The server applies the same rule — see
    // `ContentNodeDef::derive_name` in
    // crates/raisin-rocksdb/.../package_install/content_types.rs.
    //
    // A TOP-LEVEL `name:` is only an override when the file also has a
    // `properties:` block, i.e. it distinguishes structure from content.
    // Without that block the whole file IS the properties, so `name` there is a
    // property — and has to be, for a node type that requires one.
    let explicitName: string | undefined;
    if (mapped.kind === 'node-file') {
      try {
        const parsed = yaml.parse(content) || {};
        if (parsed.properties && typeof parsed.name === 'string') {
          explicitName = parsed.name;
        }
      } catch {
        // Invalid YAML — fall back to the filename-derived name
      }
    }
    const { url } = buildServerUrl(config, filePath, explicitName);
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
      // The node doesn't exist yet — create it via POST to its parent.
      //
      // This used to be gated on `filename === '.node.yaml'`, so a flat
      // `{name}.yaml` node (a layout the server installer accepts, see
      // zip_collector.rs) 404'd on every push and could never be created at
      // all. `.node.yml` failed the same way. Key off the classification
      // instead, which already knows both spellings.
      if (
        response.status === 404 &&
        (mapped.kind === 'node-yaml' || mapped.kind === 'node-file')
      ) {
        return createNodeFromYaml(
          filePath,
          content,
          options,
          token,
          timestamp,
          explicitName
        );
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

      // A local file with {env:...} tokens must not be overwritten: the server
      // only ever saw the RESOLVED value, so writing it back would bake this
      // environment's URL into the package permanently — and it looks like an
      // ordinary content change in the diff.
      const existing = fs.readFileSync(fullPath, 'utf-8');
      if (hasEnvTokens(existing)) {
        return {
          success: false,
          path: filePath,
          operation: 'pull',
          error:
            'local file contains {env:...} tokens; pull would replace them with ' +
            'this environment\'s resolved values (use --force to overwrite anyway)',
          timestamp,
        };
      }
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
/**
 * Push order within one batch.
 *
 * Asset metadata (`.node.logo.png.yaml`) updates a node the BINARY creates, so
 * it has to go last or a first-time sync fails with "no asset node yet" purely
 * because the watcher happened to report the files in that order.
 */
function pushOrder(change: ChangeEvent): number {
  if (change.type === 'unlink' || change.type === 'unlinkDir') return 0;
  return mapChangeToNode(change.path).kind === 'asset-metadata' ? 1 : 0;
}

export async function processLocalChanges(
  changes: ChangeEvent[],
  options: SyncOperationOptions
): Promise<SyncResult[]> {
  const results: SyncResult[] = [];

  // Stable sort: everything keeps its relative order except asset metadata,
  // which moves behind the binaries it describes.
  const ordered = [...changes].sort((a, b) => pushOrder(a) - pushOrder(b));

  for (const change of ordered) {
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
