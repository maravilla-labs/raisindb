/**
 * What a pull covers, and which local file stands for which server node.
 *
 * SCOPE. By default a pull refreshes what the package CONTAINS: for every
 * workspace directory under `content/`, the top-most nodes the package defines
 * there ("anchors") and everything below them, at any depth. Studio's own nodes
 * in a shared workspace (its blueprints in `stories`, its functions) are not
 * pulled into a customer package just because they share the workspace.
 * `pull.roots` in `.raisindb-cli.yaml`, or `--path ws:/path`, replaces the
 * default — `sponsoren:/` is a whole workspace.
 *
 * POLICY. The package's `.raisin-sync.yaml` (its install policy) is read the
 * way the server's installer reads it — `/{workspace}{path}`, filters
 * evaluated last-match-wins — and three things follow:
 *   - `direction: push_only | local_only` → never pulled;
 *   - `include` / `exclude` → honoured;
 *   - `mode: replace` → the PACKAGE owns that path (a deploy overwrites it),
 *     so the package is its source and pull leaves it alone unless asked
 *     (`--include-owned`). What pull is for is the content the installation
 *     owns — `skip` paths, which only an editor changes.
 */

import fs from 'fs';
import path from 'path';
import yaml from 'yaml';
import { decodeNamespace, encodeNamespace } from '../../namespace-encoding.js';
import { mapChangeToNode } from '../mapping.js';

/** Workspace + workspace-relative node path ("/a/b"). */
export interface Root {
  workspace: string;
  path: string;
}

export function nodeKey(workspace: string, nodePath: string): string {
  return `${workspace}\u0000${normalizeNodePath(nodePath)}`;
}

/** "/a/b" form, "/" for the workspace root. */
export function normalizeNodePath(p: string): string {
  const trimmed = p.replace(/^\/+/, '').replace(/\/+$/, '');
  return `/${trimmed}`;
}

/** Is `nodePath` at or below `root`? Segment-aware: /ab is not under /a. */
export function isUnder(nodePath: string, root: string): boolean {
  if (root === '/') return true;
  return nodePath === root || nodePath.startsWith(`${root}/`);
}

/** Parse "ws:/path" (or "ws" for the whole workspace). */
export function parseRootSpec(spec: string): Root {
  const s = spec.trim();
  // A workspace name may itself contain a colon (raisin:access_control), so
  // split at the colon that starts the path.
  const m = /^(.+?):(\/.*)$/.exec(s);
  if (m) return { workspace: m[1], path: normalizeNodePath(m[2]) };
  return { workspace: s, path: '/' };
}

export function formatRoot(r: Root): string {
  return `${r.workspace}:${r.path}`;
}

// ---------------------------------------------------------------------------
// Local index
// ---------------------------------------------------------------------------

/** The local files that stand for server nodes, by node key. */
export interface LocalIndex {
  /** The node's own definition: `.node.yaml`, `{name}.yaml` or `SKILL.md` */
  base: Map<string, string>;
  /** A binary / code asset file */
  binary: Map<string, string>;
  /** `.node.{file}.yaml` asset metadata */
  meta: Map<string, string>;
  /** `${nodeKey}\u0000${locale}` → translation file */
  translation: Map<string, string>;
  /** Every content-relative file the index considered, by its node key */
  fileNode: Map<string, string>;
  /** Workspace (logical name) → its directory name under the content base */
  workspaceDirs: Map<string, string>;
}

/** Recursively list files under `dir`, content-relative, posix separators. */
function walk(dir: string, rel: string, out: string[], skip: (rel: string, isDir: boolean) => boolean): void {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const childRel = rel ? `${rel}/${entry.name}` : entry.name;
    if (entry.isDirectory()) {
      if (skip(childRel, true)) continue;
      walk(path.join(dir, entry.name), childRel, out, skip);
    } else if (entry.isFile()) {
      if (skip(childRel, false)) continue;
      if (entry.name.endsWith('.raisindb-partial')) continue;
      out.push(childRel);
    }
  }
}

const NON_CONTENT_DIRS = new Set(['nodetypes', 'archetypes', 'elementtypes', 'mixins', 'workspaces', 'static']);

/**
 * Index the content base. `isIgnored` takes a content-relative path.
 * Files the mapping does not recognise as node files (`.gitkeep`, a README)
 * are not indexed — pull neither writes nor deletes them.
 */
export function buildLocalIndex(
  contentBase: string,
  isIgnored: (rel: string) => boolean,
  legacyLayout: boolean
): LocalIndex {
  const index: LocalIndex = {
    base: new Map(),
    binary: new Map(),
    meta: new Map(),
    translation: new Map(),
    fileNode: new Map(),
    workspaceDirs: new Map(),
  };
  if (!fs.existsSync(contentBase)) return index;

  for (const entry of fs.readdirSync(contentBase, { withFileTypes: true })) {
    if (!entry.isDirectory() || entry.name.startsWith('.')) continue;
    if (legacyLayout && NON_CONTENT_DIRS.has(entry.name)) continue;
    if (isIgnored(entry.name)) continue;
    index.workspaceDirs.set(decodeNamespace(entry.name), entry.name);
  }

  const files: string[] = [];
  for (const dirName of index.workspaceDirs.values()) {
    walk(path.join(contentBase, dirName), dirName, files, (rel) => isIgnored(rel));
  }

  for (const rel of files) {
    const mapped = mapChangeToNode(rel);
    if (!mapped.workspace || mapped.nodePath === undefined) continue;
    const key = nodeKey(mapped.workspace, mapped.nodePath);
    switch (mapped.kind) {
      case 'node-yaml':
      case 'node-file':
        // A directory may hold both `.node.yaml` and (for skills) SKILL.md;
        // the installer prefers SKILL.md, so does the index.
        if (!index.base.has(key) || mapped.skillMd) index.base.set(key, rel);
        break;
      case 'code':
      case 'asset':
        index.binary.set(key, rel);
        break;
      case 'asset-metadata':
        index.meta.set(key, rel);
        break;
      case 'translation':
        index.translation.set(`${key}\u0000${mapped.locale}`, rel);
        break;
      default:
        continue;
    }
    index.fileNode.set(rel, key);
  }
  return index;
}

/** The directory a workspace's files live in (existing, or its encoded name). */
export function workspaceDir(index: LocalIndex, workspace: string): string {
  return index.workspaceDirs.get(workspace) ?? encodeNamespace(workspace);
}

/**
 * The default roots: per workspace directory, the top-most nodes the package
 * defines. A directory with no node file of its own is only a path segment
 * (`apps/local/` holding `apps/local/appdefs/x/.node.yaml`), so the anchors
 * are found below it.
 */
export function defaultRoots(index: LocalIndex): Root[] {
  const byWorkspace = new Map<string, Set<string>>();
  const binaryKeys = new Set(index.binary.keys());
  for (const key of new Set(index.fileNode.values())) {
    const [workspace, nodePath] = key.split('\u0000');
    if (!byWorkspace.has(workspace)) byWorkspace.set(workspace, new Set());
    // A bare file (an image dropped into `assets/website/teams/`) defines no
    // container, so it anchors at its top-level folder: an upload on the live
    // server next to it is then part of the pull, not "outside the package".
    // A file below a node-defining directory is covered by that directory.
    const segments = nodePath.split('/').filter(Boolean);
    const definedAncestor = segments
      .slice(0, -1)
      .some((_, i) => index.base.has(nodeKey(workspace, `/${segments.slice(0, i + 1).join('/')}`)));
    const bare = binaryKeys.has(key) && !index.base.has(key) && !definedAncestor;
    const anchor = bare ? `/${segments[0]}` : nodePath;
    byWorkspace.get(workspace)!.add(anchor);
  }
  const roots: Root[] = [];
  for (const [workspace, paths] of byWorkspace) {
    const sorted = [...paths].sort();
    const anchors: string[] = [];
    for (const p of sorted) {
      if (p === '/') continue;
      if (anchors.some((a) => isUnder(p, a))) continue;
      anchors.push(p);
    }
    for (const a of anchors) roots.push({ workspace, path: a });
  }
  return roots.sort((a, b) => formatRoot(a).localeCompare(formatRoot(b)));
}

// ---------------------------------------------------------------------------
// Install policy (.raisin-sync.yaml)
// ---------------------------------------------------------------------------

interface PolicyFilter {
  root: string;
  mode?: string;
  direction?: string;
  include?: string[];
  exclude?: string[];
}

export interface InstallPolicy {
  defaults: { mode?: string };
  filters: PolicyFilter[];
}

/**
 * The package's install policy, or null. A file with the legacy name that is
 * shaped like a CLI config (it has `server`) is not a policy — see config.ts.
 */
export function loadInstallPolicy(packageDir: string): InstallPolicy | null {
  const file = path.join(packageDir, '.raisin-sync.yaml');
  if (!fs.existsSync(file)) return null;
  const doc = yaml.parse(fs.readFileSync(file, 'utf-8')) as Record<string, unknown> | null;
  if (!doc || typeof doc !== 'object' || typeof doc.server === 'string') return null;
  return {
    defaults: (doc.defaults as { mode?: string }) ?? {},
    filters: Array.isArray(doc.filters) ? (doc.filters as PolicyFilter[]).filter((f) => f && typeof f.root === 'string') : [],
  };
}

/** Port of glob_matches in raisin-packages/src/sync_config/filter.rs. */
export function policyGlob(pattern: string, p: string): boolean {
  const pat = pattern.split('/');
  const parts = p.split('/').filter((s) => s.length > 0);
  const seg = (pt: string, s: string): boolean => {
    const re = new RegExp(`^${pt.replace(/[.+^${}()|[\]\\]/g, '\\$&').replace(/\*/g, '.*').replace(/\?/g, '.')}$`);
    return re.test(s);
  };
  const match = (pi: number, si: number): boolean => {
    if (pi === pat.length) return si === parts.length;
    if (pat[pi] === '**') {
      if (pi === pat.length - 1) return true;
      for (let i = si; i <= parts.length; i++) if (match(pi + 1, i)) return true;
      return false;
    }
    if (si === parts.length) return false;
    return seg(pat[pi], parts[si]) && match(pi + 1, si + 1);
  };
  return match(0, 0);
}

export type PolicyVerdict =
  | { pull: true; owned: boolean; ownedBy?: string }
  | { pull: false; reason: string };

/**
 * What the install policy says about pulling `workspace`:`nodePath`. Mirrors
 * the server: plain string prefix on `/{workspace}{path}`, last match wins,
 * and with no matching filter `defaults.mode` (whose server default is
 * `replace`) decides ownership.
 */
export function policyFor(policy: InstallPolicy | null, workspace: string, nodePath: string): PolicyVerdict {
  if (!policy) return { pull: true, owned: false };
  const full = `/${workspace}${nodePath === '/' ? '' : nodePath}`;
  const reversed = [...policy.filters].reverse();

  for (const f of reversed) {
    if (!full.startsWith(f.root) || !f.direction) continue;
    if (f.direction === 'push_only' || f.direction === 'local_only') {
      return { pull: false, reason: `direction: ${f.direction} (filter '${f.root}')` };
    }
    break;
  }

  const scoped = reversed.find((f) => full.startsWith(f.root));
  if (scoped) {
    const rel = full.slice(scoped.root.length).replace(/^\/+/, '');
    if (scoped.include?.length && !scoped.include.some((g) => policyGlob(g, rel))) {
      return { pull: false, reason: `not in include of filter '${scoped.root}'` };
    }
    if (scoped.exclude?.some((g) => policyGlob(g, rel))) {
      return { pull: false, reason: `excluded by filter '${scoped.root}'` };
    }
  }

  const modeFilter = reversed.find((f) => full.startsWith(f.root) && f.mode);
  const mode = modeFilter?.mode ?? policy.defaults.mode ?? 'replace';
  if (mode === 'replace') {
    return { pull: true, owned: true, ownedBy: modeFilter ? `filter '${modeFilter.root}'` : 'defaults' };
  }
  return { pull: true, owned: false };
}
