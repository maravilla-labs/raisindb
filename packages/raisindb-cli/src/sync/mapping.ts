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
 *                               `.yaml` only — a bare `{name}.yml` is an asset
 * - `{dir}/index.js|.py|.star`→ code asset child node `{dir}/index.js`
 * - other non-YAML files      → binary asset child node (full filename)
 * - `{dir}/.node.{file}.yaml` → metadata for the sibling asset `{dir}/{file}`
 * - `.node.{locale}.yaml` / `{name}.{locale}.yaml` → translation overlay
 * - nodetypes/, archetypes/, elementtypes/, mixins/
 *                             → schema: live-synced to the management API (upsert)
 * - manifest.yaml, workspaces/, static/
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
  | 'asset-metadata' // .node.{filename}.yaml — title/description/props for a sibling asset
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

/**
 * Code file extensions pushed as inline `code` property on asset nodes.
 *
 * `.wasm` is deliberately NOT here. A WebAssembly component is BYTES: it must
 * travel as a binary asset (multipart upload), and inlining it as a `code`
 * string would mangle it at the first non-UTF-8 byte. A wasm function's
 * `.node.yaml` carries `language: wasm` and an `entry_file` naming the sibling
 * artifact, which the `asset` branch below uploads unchanged.
 * Test: `mapping.test.ts` — "a .wasm artifact is an asset, never code".
 */
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

  // Asset metadata: `.node.{filename}.yaml` carries the title/description/extra
  // properties for the sibling file `{filename}` (e.g. `.node.logo.png.yaml`
  // describes `logo.png`). The `.node.` prefix is what makes it metadata rather
  // than a node named `logo.png` — see `parse_asset_metadata_filename` in
  // crates/raisin-rocksdb/.../package_install/content_types.rs.
  //
  // Translations (`.node.de.yaml`) and the folder definition (`.node.yaml`) are
  // both matched above, so anything still here names a sibling file.
  if (filename.startsWith('.node.') && filename.endsWith('.yaml')) {
    const target = filename.slice('.node.'.length, -'.yaml'.length);
    if (target) {
      return {
        kind: 'asset-metadata',
        workspace,
        nodePath: [...rest.slice(0, -1), target].join('/'),
      };
    }
  }

  // Any other dot-prefixed file
  if (filename.startsWith('.node.')) {
    return { kind: 'skip', reason: 'unrecognised .node.* file' };
  }

  // Hidden files
  if (filename.startsWith('.')) {
    return { kind: 'skip', reason: 'hidden file' };
  }

  const ext = extOf(filename);

  // Named node YAML: {name}.yaml → node {name} (extension stripped).
  //
  // `.yaml` ONLY, matching the server installer (`zip_collector.rs`). A bare
  // `{name}.yml` is NOT a node declaration — it falls through to the asset
  // branch below and installs as a `raisin:Asset`, which is how a data file
  // gets into a package. This used to accept `.yml` too, so the same file was
  // a node here and an asset on the server.
  //
  // `.node.yml` IS a node (handled above): the `.node.` prefix is the
  // declaration, not the extension.
  if (ext === '.yaml') {
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
