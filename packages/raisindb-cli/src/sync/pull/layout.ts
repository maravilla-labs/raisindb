/**
 * Server nodes → the files a package holds for them.
 *
 * The layout is the one `raisindb package create` packs and the server
 * installer reads (crates/raisin-rocksdb/.../package_install, and
 * `sync/mapping.ts` on this side), so a pulled folder deploys back unchanged:
 *
 *   node                 → {ws}/{path}/.node.yaml   (or the file it already has:
 *                          `{name}.yaml`, or SKILL.md for a raisin:Skill)
 *   asset (has a file)   → {ws}/{parent}/{name}     the bytes, streamed
 *                          {ws}/{parent}/.node.{name}.yaml   only when the asset
 *                          carries more than the installer derives by itself
 *   translation overlay  → .node.{locale}.yaml beside the node's file
 *                          ({name}.{locale}.yaml for a `{name}.yaml` node)
 *
 * Workspace directories use the filesystem encoding (raisin:access_control →
 * _raisin__access_control). Nodes BELOW an asset (renditions, thumbnails) are
 * derived by the server and have no place in the layout; they are counted and
 * skipped.
 */

import type { RemoteNode } from './remote.js';
import { isSafeLocale, isSafeWorkspace, safeNodeSegments, segmentProblem, UnsafePathError } from './safe-path.js';
import type { LocalIndex } from './scope.js';
import { nodeKey, workspaceDir } from './scope.js';
import {
  canonicalHash,
  cleanProperties,
  isPlainObject,
  isServerOwnedKey,
  portable,
  type Json,
  type JsonObject,
} from './normalize.js';

export type DesiredKind = 'node' | 'skill' | 'asset-meta' | 'translation' | 'binary';

interface DesiredBase {
  /** Content-relative path, posix separators */
  rel: string;
  kind: DesiredKind;
  workspace: string;
  nodePath: string;
  key: string;
  /** Identity of the server's version — changes exactly when the server's does */
  serverHash: string;
  updatedAt?: string;
}

export interface DesiredText extends DesiredBase {
  kind: 'node' | 'skill' | 'asset-meta' | 'translation';
  /** What the file must say (node file object, overlay, or asset metadata) */
  value: JsonObject;
  locale?: string;
}

export interface DesiredBinary extends DesiredBase {
  kind: 'binary';
  /** sha256 hex of the bytes when the server knows it (`content_hash`) */
  contentHash: string | null;
  size: number | null;
  /** Inline function code (`code` property) wins over the stored file */
  inlineCode: string | null;
}

export type DesiredFile = DesiredText | DesiredBinary;

export interface LayoutResult {
  files: DesiredFile[];
  /** Nodes below an asset — server-derived, not part of a package */
  derivedSkipped: number;
  /** Bare folders the installer creates by itself — the directory is enough */
  implicitFolders: number;
  /** Server entries whose names are not safe file names — never written */
  refused: Array<{ what: string; reason: string }>;
}

/**
 * A `raisin:Folder` with nothing of its own is what the installer creates for
 * a directory that has no `.node.yaml`. Writing one would add a file the
 * package never had; the directory already says it.
 */
export function isImplicitFolder(node: RemoteNode): boolean {
  if (node.node_type !== 'raisin:Folder' || node.archetype) return false;
  const props = cleanProperties(node.properties);
  const keys = Object.keys(props);
  return keys.length === 0 || (keys.length === 1 && props.title === node.name);
}

/** A node whose bytes are the point: it has a stored file or inline code. */
export function isAssetNode(node: RemoteNode): boolean {
  const file = node.properties.file;
  if (isPlainObject(file)) {
    const meta = file.metadata;
    if (isPlainObject(meta) && typeof meta.storage_key === 'string') return true;
    if (typeof file.url === 'string') return true;
  }
  return node.node_type === 'raisin:Asset' && typeof node.properties.code === 'string';
}

/** Properties the installer sets on an asset by itself (install_binary_file). */
const ASSET_DERIVED = new Set(['file', 'file_type', 'file_size', 'content_hash', 'code', 'title']);

/**
 * Properties the server's processing derives from the bytes (renditions,
 * dimensions, …). They are regenerated on every install, and pulling them
 * would freeze one server's job state — including its failures — into the
 * package.
 */
const ASSET_PROCESSED = /^(renditions?(_.*)?|thumbnails?(_.*)?|width|height|duration|exif|blurhash|dominant_colou?rs?)$/;

/**
 * The `.node.{file}.yaml` an asset needs, or null when it needs none. It
 * always names `node_type`: `package create --check` requires it.
 */
export function assetMetadata(node: RemoteNode): JsonObject | null {
  const out: JsonObject = { node_type: node.node_type };
  const title = node.properties.title;
  if (typeof title === 'string' && title !== node.name) out.title = title;
  if (typeof node.properties.description === 'string') out.description = node.properties.description;
  const extra: JsonObject = {};
  for (const [k, v] of Object.entries(node.properties)) {
    if (ASSET_DERIVED.has(k) || ASSET_PROCESSED.test(k) || isServerOwnedKey(k) || k === 'description') continue;
    extra[k] = portable(v as Json);
  }
  if (Object.keys(extra).length > 0) out.properties = extra;
  const meaningful = node.node_type !== 'raisin:Asset' || Object.keys(out).length > 1;
  return meaningful ? out : null;
}

function parentOf(nodePath: string): string {
  const i = nodePath.lastIndexOf('/');
  return i <= 0 ? '' : nodePath.slice(1, i);
}

function join(...parts: string[]): string {
  return parts.filter((p) => p !== '').join('/');
}

/** The file a translation for `locale` lives in, given the node's own file. */
export function translationRel(baseRel: string, locale: string): string {
  const slash = baseRel.lastIndexOf('/');
  const dir = baseRel.slice(0, slash);
  const file = baseRel.slice(slash + 1);
  if (file === '.node.yaml' || file === 'SKILL.md') return `${dir}/.node.${locale}.yaml`;
  return `${dir}/${file.replace(/\.yaml$/, '')}.${locale}.yaml`;
}

/**
 * Lay out every node of one scan. `overlays` holds, per node key, the overlay
 * of each locale that changes something (see computeOverlay).
 */
export function layoutNodes(
  nodes: RemoteNode[],
  index: LocalIndex,
  overlays: Map<string, Map<string, JsonObject>>
): LayoutResult {
  const files: DesiredFile[] = [];
  let derivedSkipped = 0;
  let implicitFolders = 0;
  const refused: Array<{ what: string; reason: string }> = [];

  const byWs = new Map<string, RemoteNode[]>();
  for (const n of nodes) {
    if (!byWs.has(n.workspace)) byWs.set(n.workspace, []);
    byWs.get(n.workspace)!.push(n);
  }

  for (const [workspace, wsNodes] of byWs) {
    if (!isSafeWorkspace(workspace) || segmentProblem(workspaceDir(index, workspace))) {
      refused.push({ what: `workspace ${JSON.stringify(workspace)}`, reason: 'not a safe directory name' });
      continue;
    }
    const dir = workspaceDir(index, workspace);
    const assetPaths = new Set(wsNodes.filter(isAssetNode).map((n) => n.path));
    const parents = new Set(wsNodes.map((n) => n.path.slice(0, n.path.lastIndexOf('/'))));
    const underAsset = (p: string) => {
      for (let i = p.lastIndexOf('/'); i > 0; i = p.lastIndexOf('/', i - 1)) {
        if (assetPaths.has(p.slice(0, i))) return true;
      }
      return false;
    };

    for (const node of wsNodes) {
      if (underAsset(node.path)) {
        derivedSkipped++;
        continue;
      }
      // Every segment of the path, and the name, become file names.
      try {
        safeNodeSegments(node.path);
        const problem = segmentProblem(node.name);
        if (problem) throw new UnsafePathError(`${problem}: name ${JSON.stringify(node.name)}`);
        if (node.name !== node.path.slice(node.path.lastIndexOf('/') + 1)) {
          throw new UnsafePathError(`name ${JSON.stringify(node.name)} does not match its path`);
        }
      } catch (err) {
        refused.push({ what: `${workspace}:${JSON.stringify(node.path)}`, reason: (err as Error).message });
        continue;
      }
      const key = nodeKey(workspace, node.path);
      const common = { workspace, nodePath: node.path, key, updatedAt: node.updated_at };

      if (isAssetNode(node)) {
        const props = node.properties;
        const inlineCode = typeof props.code === 'string' ? props.code : null;
        const contentHash = typeof props.content_hash === 'string' ? props.content_hash : null;
        const file = isPlainObject(props.file) ? props.file : {};
        const meta = isPlainObject(file.metadata) ? file.metadata : {};
        const size = typeof props.file_size === 'number' ? props.file_size
          : typeof file.size === 'number' ? file.size : null;
        files.push({
          ...common,
          kind: 'binary',
          rel: index.binary.get(key) ?? join(dir, node.path.slice(1)),
          contentHash: inlineCode !== null ? null : contentHash,
          size,
          inlineCode,
          serverHash: canonicalHash({
            code: inlineCode,
            hash: contentHash,
            key: meta.storage_key ?? file.url ?? null,
            size,
          }),
        });
        const metadata = assetMetadata(node);
        const metaRel = index.meta.get(key) ?? join(dir, parentOf(node.path), `.node.${node.name}.yaml`);
        // A metadata file the package already has is kept current even when
        // the asset no longer needs one — deleting it is a deletion decision.
        if (metadata || index.meta.has(key)) {
          const value = metadata ?? {};
          files.push({ ...common, kind: 'asset-meta', rel: metaRel, value, serverHash: canonicalHash(value) });
        }
        continue;
      }

      const existing = index.base.get(key);
      // Only when something below it is pulled: an EMPTY folder exists in a
      // package only through its .node.yaml.
      if (!existing && parents.has(node.path) && isImplicitFolder(node)) {
        implicitFolders++;
        continue;
      }
      const isSkill = node.node_type === 'raisin:Skill';
      const baseRel =
        existing ?? join(dir, node.path.slice(1), isSkill ? 'SKILL.md' : '.node.yaml');
      const kind = baseRel.endsWith('/SKILL.md') || baseRel === 'SKILL.md' ? 'skill' : 'node';
      const value: JsonObject = { node_type: node.node_type, properties: cleanProperties(node.properties) };
      if (node.archetype) value.archetype = node.archetype;
      files.push({ ...common, kind, rel: baseRel, value, serverHash: canonicalHash(value) });

      for (const [locale, overlay] of overlays.get(key) ?? []) {
        if (!isSafeLocale(locale)) {
          refused.push({ what: `${workspace}:${node.path} [${JSON.stringify(locale)}]`, reason: 'not a locale code' });
          continue;
        }
        files.push({
          ...common,
          kind: 'translation',
          locale,
          rel: index.translation.get(`${key}\u0000${locale}`) ?? translationRel(baseRel, locale),
          value: overlay,
          serverHash: canonicalHash(overlay),
        });
      }
    }
  }
  return { files, derivedSkipped, implicitFolders, refused };
}
