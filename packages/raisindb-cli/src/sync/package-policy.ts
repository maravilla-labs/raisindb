/**
 * The PACKAGE's install policy (`.raisin-sync.yaml` beside `manifest.yaml`),
 * read by `sync` so a push does not do what `deploy --install` refuses to.
 *
 * The server's installer leaves an existing node alone on a path the package
 * marks `skip` — that is how a package says "editors own this once it exists".
 * `sync --push` / `--watch` used to PUT every file regardless, so a developer
 * running watch mode against a live repository reverted every editor change on
 * those paths (watch mode starts with a push of EVERY local file).
 *
 * The CLI's own sync config still decides what gets pushed. The policy only
 * makes overwriting a protected node a decision someone has to take: a prompt
 * when interactive, `--ignore-package-policy` when headless.
 */

import fs from 'fs';
import path from 'path';
import yaml from 'yaml';

export type PackageSyncMode = 'skip' | 'replace' | 'merge' | 'update';

export interface PackagePolicy {
  defaults: { mode: PackageSyncMode };
  filters: Array<{ root: string; mode?: PackageSyncMode }>;
}

export interface PathPolicy {
  mode: PackageSyncMode;
  /** The filter root that decided it, or null for the top-level default. */
  source: string | null;
}

const POLICY_FILENAME = '.raisin-sync.yaml';
const MODES: readonly PackageSyncMode[] = ['skip', 'replace', 'merge', 'update'];

function asMode(value: unknown): PackageSyncMode | undefined {
  return typeof value === 'string' && (MODES as readonly string[]).includes(value)
    ? (value as PackageSyncMode)
    : undefined;
}

/**
 * Parse a policy document. Returns null for anything that is not an install
 * policy — notably a legacy CLI config of the same name (it has `server`; see
 * `looksLikeCliConfig` in config.ts).
 */
export function parsePackagePolicy(doc: unknown): PackagePolicy | null {
  if (!doc || typeof doc !== 'object') return null;
  const d = doc as Record<string, unknown>;
  if (typeof d.server === 'string') return null;
  if (!('defaults' in d) && !('filters' in d)) return null;

  const defaults = (d.defaults ?? {}) as Record<string, unknown>;
  const filters = Array.isArray(d.filters) ? d.filters : [];
  return {
    // Rust: `SyncMode::default()` is `Replace`.
    defaults: { mode: asMode(defaults.mode) ?? 'replace' },
    filters: filters
      .filter((f): f is Record<string, unknown> => !!f && typeof f === 'object')
      .filter((f) => typeof f.root === 'string')
      .map((f) => ({ root: f.root as string, mode: asMode(f.mode) })),
  };
}

/** Load `<packageDir>/.raisin-sync.yaml` if it is an install policy. */
export function loadPackagePolicy(packageDir: string): PackagePolicy | null {
  const file = path.join(packageDir, POLICY_FILENAME);
  if (!fs.existsSync(file)) return null;
  try {
    return parsePackagePolicy(yaml.parse(fs.readFileSync(file, 'utf-8')));
  } catch (e) {
    console.warn(
      `Warning: could not parse ${file} (${e instanceof Error ? e.message : String(e)}); ` +
        'the package install policy is not applied to this sync.'
    );
    return null;
  }
}

/**
 * Resolve the mode for one node — a port of the server's
 * `SyncConfig::get_mode_and_source_for_path`
 * (crates/raisin-packages/src/sync_config/filter.rs) and the path it is called
 * with in `resolve_install_policy_for_path` (`/{workspace}{node_path}`).
 * Keep the two identical: LAST matching filter wins, matching is a plain
 * prefix test on `root`, a filter without `mode` falls through.
 */
export function policyForPath(
  policy: PackagePolicy,
  workspace: string,
  nodePath: string
): PathPolicy {
  const rel = nodePath.replace(/^\/+/, '');
  const full = `/${workspace}/${rel}`;
  for (let i = policy.filters.length - 1; i >= 0; i--) {
    const filter = policy.filters[i];
    if (full.startsWith(filter.root) && filter.mode) {
      return { mode: filter.mode, source: filter.root };
    }
  }
  return { mode: policy.defaults.mode, source: null };
}

/** Does this policy protect any path at all? */
export function hasSkipPaths(policy: PackagePolicy | null): boolean {
  if (!policy) return false;
  return policy.defaults.mode === 'skip' || policy.filters.some((f) => f.mode === 'skip');
}

export function describePolicy(p: PathPolicy): string {
  return p.source
    ? `package policy: ${p.mode} (filter '${p.source}')`
    : `package policy: ${p.mode} (default)`;
}
