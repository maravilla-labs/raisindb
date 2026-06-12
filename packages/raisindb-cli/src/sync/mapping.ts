/**
 * Pure change → node mapping for package-dir sync.
 *
 * Maps a file path (relative to the package content base, e.g.
 * "functions/lib/shiftboard/list-shifts/index.js") to the server node it
 * belongs to. The rules mirror the server-side package installer
 * (crates/raisin-rocksdb/src/jobs/handlers/package_install):
 *
 * - `{dir}/.node.yaml`        → the node for `{dir}` itself
 * - `{dir}/{name}.yaml`       → node `{dir}/{name}` (extension stripped)
 * - `{dir}/index.js|.py|.star`→ code asset child node `{dir}/index.js`
 * - other non-YAML files      → binary asset child node (full filename)
 * - `.node.{locale}.yaml` / `{name}.{locale}.yaml` → translation overlay
 * - manifest.yaml, nodetypes/, workspaces/, mixins/, archetypes/
 *                             → structural: applied at install time only,
 *                               requires a re-deploy (deploy --install)
 */

import { decodeNamespace } from '../namespace-encoding.js';

/** Which schema kind a file under a schema directory defines */
export type SchemaKind = 'nodetype' | 'archetype' | 'elementtype' | 'mixin';

/** What kind of change a file represents */
export type ChangeKind =
  | 'node-yaml' // .node.yaml describing its containing directory node
  | 'node-file' // {name}.yaml describing a named node
  | 'code' // .js / .py / .star function code (pushed as inline `code` property)
  | 'asset' // other binary/asset file (pushed via multipart upload)
  | 'translation' // {base}.{locale}.yaml translation overlay
  | 'schema' // nodetype/archetype/elementtype/mixin → management API (upsert)
  | 'structural' // manifest / workspaces — needs re-deploy
  | 'skip'; // not a syncable content file

export interface MappedChange {
  kind: ChangeKind;
  /** Workspace name (namespace-decoded), set for content changes */
  workspace?: string;
  /** Node path within the workspace, no leading slash */
  nodePath?: string;
  /** Locale for translation files */
  locale?: string;
  /** Schema kind, set when kind === 'schema' */
  schemaKind?: SchemaKind;
  /** Human-readable hint for structural/skip changes */
  reason?: string;
}

/** Code file extensions pushed as inline `code` property on asset nodes */
export const CODE_EXTENSIONS = ['.js', '.py', '.star'];

/**
 * Top-level package directories holding schema definitions. These are synced
 * live to the management API (upsert), unlike content nodes which go to the
 * repository workspace.
 */
const SCHEMA_DIRS: Record<string, SchemaKind> = {
  nodetypes: 'nodetype',
  archetypes: 'archetype',
  elementtypes: 'elementtype',
  mixins: 'mixin',
};

/** Top-level package directories that are install-time-only (structural) */
const STRUCTURAL_DIRS = new Set(['workspaces', 'static']);

const LOCALE_RE = /^[a-zA-Z]{2,3}(-[a-zA-Z]{2,4}|\d{3})?$/;

/**
 * Parse a translation locale from a YAML filename.
 *
 * `.node.de.yaml` → "de"; `about.de.yaml` → "de"; `.node.yaml` → null;
 * `.node.index.js.yaml` → null (asset metadata, not a translation).
 */
export function parseTranslationLocale(filename: string): string | null {
  if (!filename.endsWith('.yaml')) return null;

  const withoutYaml = filename.slice(0, -'.yaml'.length);

  if (filename.startsWith('.node.')) {
    const inner = withoutYaml.slice('.node.'.length);
    if (!inner) return null;
    return LOCALE_RE.test(inner) ? inner : null;
  }

  const dotPos = withoutYaml.lastIndexOf('.');
  if (dotPos < 0) return null;
  const candidate = withoutYaml.slice(dotPos + 1);
  if (!candidate) return null;
  return LOCALE_RE.test(candidate) ? candidate : null;
}

/** Get the file extension including the dot, lowercased ("" if none). */
function extOf(filename: string): string {
  const dot = filename.lastIndexOf('.');
  return dot > 0 ? filename.slice(dot).toLowerCase() : '';
}

/**
 * Map a changed file (relative path, posix separators) to its target node.
 *
 * @param relPath - path relative to the content base (content/ dir if present)
 * @param explicitName - optional `name`/`properties.name` parsed from the YAML,
 *                       which overrides the filename-derived node name
 */
export function mapChangeToNode(
  relPath: string,
  explicitName?: string
): MappedChange {
  const normalized = relPath.split('\\').join('/').replace(/^\/+/, '');
  if (!normalized) return { kind: 'skip', reason: 'empty path' };

  const parts = normalized.split('/');
  const filename = parts[parts.length - 1];

  // Root-level structural files
  if (parts.length === 1) {
    if (filename === 'manifest.yaml' || filename === 'manifest.yml') {
      return {
        kind: 'structural',
        reason: 'package manifest changed — run deploy --install to apply',
      };
    }
    return { kind: 'skip', reason: 'not inside a workspace directory' };
  }

  // Schema directories (nodetypes/archetypes/elementtypes/mixins) → synced
  // live to the management API. Only .yaml/.yml definition files apply.
  const schemaKind = SCHEMA_DIRS[parts[0]];
  if (schemaKind) {
    if (filename.endsWith('.yaml') || filename.endsWith('.yml')) {
      return { kind: 'schema', schemaKind };
    }
    return { kind: 'skip', reason: 'non-YAML file in a schema directory' };
  }

  // Structural directories (only meaningful when content base == package dir)
  if (STRUCTURAL_DIRS.has(parts[0])) {
    return {
      kind: 'structural',
      reason: `${parts[0]}/ definitions are applied at install time — run deploy --install to apply`,
    };
  }

  const workspace = decodeNamespace(parts[0]);
  const rest = parts.slice(1);

  // Translation overlay files
  const locale = parseTranslationLocale(filename);
  if (locale) {
    let nodePath: string;
    if (filename.startsWith('.node.')) {
      nodePath = rest.slice(0, -1).join('/');
    } else {
      const withoutYaml = filename.slice(0, -'.yaml'.length);
      const base = withoutYaml.slice(0, withoutYaml.lastIndexOf('.'));
      nodePath = [...rest.slice(0, -1), base].join('/');
    }
    return { kind: 'translation', workspace, nodePath, locale };
  }

  // .node.yaml → the containing directory's node
  if (filename === '.node.yaml' || filename === '.node.yml') {
    return {
      kind: 'node-yaml',
      workspace,
      nodePath: rest.slice(0, -1).join('/'),
    };
  }

  // Asset metadata files (.node.index.js.yaml) — handled at install time
  if (filename.startsWith('.node.')) {
    return {
      kind: 'skip',
      reason: 'asset metadata files are applied at install time',
    };
  }

  // Hidden files
  if (filename.startsWith('.')) {
    return { kind: 'skip', reason: 'hidden file' };
  }

  const ext = extOf(filename);

  // Named node YAML: {name}.yaml → node {name} (extension stripped)
  if (ext === '.yaml' || ext === '.yml') {
    const stem = filename.slice(0, -ext.length);
    const name = explicitName || stem;
    return {
      kind: 'node-file',
      workspace,
      nodePath: [...rest.slice(0, -1), name].join('/'),
    };
  }

  // Function code files → inline `code` property on the asset child node
  if (CODE_EXTENSIONS.includes(ext)) {
    return { kind: 'code', workspace, nodePath: rest.join('/') };
  }

  // Everything else is a binary asset child node
  return { kind: 'asset', workspace, nodePath: rest.join('/') };
}
