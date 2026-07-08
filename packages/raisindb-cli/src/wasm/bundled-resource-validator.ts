/**
 * Validate that internal (bundled) Resource properties on content nodes have
 * their binary actually shipped in the package.
 *
 * Mirrors the server's package-install binding (`build_content_entries` /
 * `resource_ref_filename` in `raisin-rocksdb`): on install, a binary sitting
 * beside a `.node.yaml` and referenced by one of that node's authored, internal
 * `Resource` properties is ingested into blob storage and the Resource is
 * rebound to it. If the referenced binary is NOT bundled, the server cannot
 * ingest anything and the Resource is left pointing at a storage key with no
 * bytes — downloads/displays 404. This check catches that at package-create
 * time (as a warning, so genuinely-external or server-side resources don't
 * block packaging).
 */

import * as fs from 'fs';
import * as path from 'path';
import yaml from 'yaml';
import type {
  PackageValidationResults,
  ValidationError,
  ValidationResult,
} from './types.js';

const NODE_FILE = /^\.node\.ya?ml$/i;

/**
 * A Resource reference is a *bundled* binary only when it is a bare filename
 * (no path separators, no URL scheme) — i.e. a sibling of the `.node.yaml`.
 * Absolute URLs, nested paths, and server blob keys are not package-relative.
 */
function bundledFilename(ref: unknown): string | null {
  if (typeof ref !== 'string' || ref.length === 0) return null;
  if (ref.includes('/') || ref.includes('\\') || ref.includes('://')) return null;
  return ref;
}

/**
 * If `value` is a top-level internal `Resource`, return the bundle-relative
 * filename it points at (mirroring the server's `storage_key` → `url` → `name`
 * precedence). Returns null for non-Resources, external resources, or
 * non-bundle references.
 */
function resourceBundledFilename(value: unknown): string | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  const v = value as Record<string, unknown>;
  if (v.is_external === true) return null;

  const metadata =
    v.metadata && typeof v.metadata === 'object'
      ? (v.metadata as Record<string, unknown>)
      : undefined;
  const ref = metadata?.storage_key ?? v.url ?? v.name;
  return bundledFilename(ref);
}

/**
 * Walk `content/` for `.node.yaml` files and warn when a top-level internal
 * Resource property references a bundled binary that isn't present beside it.
 */
export function validateBundledResources(
  packageDir: string
): PackageValidationResults {
  const results: PackageValidationResults = {};

  const contentRoot = path.join(packageDir, 'content');
  if (!fs.existsSync(contentRoot)) return results;

  const walk = (dir: string): void => {
    let entries: fs.Dirent[];
    try {
      entries = fs.readdirSync(dir, { withFileTypes: true });
    } catch {
      return;
    }

    for (const entry of entries) {
      const full = path.join(dir, entry.name);
      if (entry.isDirectory()) {
        walk(full);
        continue;
      }
      if (!entry.isFile() || !NODE_FILE.test(entry.name)) continue;

      let parsed: unknown;
      try {
        parsed = yaml.parse(fs.readFileSync(full, 'utf-8'));
      } catch {
        continue; // YAML errors are reported by the schema validator.
      }

      const props =
        parsed && typeof parsed === 'object'
          ? (parsed as Record<string, unknown>).properties
          : undefined;
      if (!props || typeof props !== 'object') continue;

      const relPath = path.relative(packageDir, full);
      const warnings: ValidationError[] = [];

      for (const [key, value] of Object.entries(props as Record<string, unknown>)) {
        const filename = resourceBundledFilename(value);
        if (!filename) continue;

        if (!fs.existsSync(path.join(dir, filename))) {
          warnings.push({
            file_path: relPath,
            field_path: `properties.${key}`,
            error_code: 'MISSING_BUNDLED_BINARY',
            message:
              `Property '${key}' is an internal Resource referencing bundled binary ` +
              `'${filename}', but no such file sits beside this node. On install the ` +
              `server cannot ingest it, so the Resource will point at a storage key ` +
              `with no bytes (downloads 404). Bundle the file next to .node.yaml, or ` +
              `mark the Resource external (is_external: true) with a full url.`,
            severity: 'warning',
            fix_type: 'manual',
          });
        }
      }

      if (warnings.length > 0) {
        const result: ValidationResult = {
          success: true,
          file_type: 'content',
          errors: [],
          warnings,
        };
        results[relPath] = result;
      }
    }
  };

  walk(contentRoot);
  return results;
}
