/**
 * Every file a pull writes is named by the SERVER: node paths and names,
 * workspace names, locale codes. A compromised server, or any author on it,
 * controls those strings. Without these checks a node named
 * `../../.ssh/authorized_keys` is a write outside the package.
 *
 * Two layers, both required:
 *   1. grammar: each server-derived segment must be a plain file name — no
 *      `.`/`..`, no separators, no NUL/control characters, no drive letters,
 *      nothing that decodes to a dot segment;
 *   2. containment: the joined path must resolve inside the content base, and
 *      no existing component between the base and the target may be a
 *      symlink — so a link planted in the package cannot redirect a write or
 *      a delete.
 */

import fs from 'fs';
import path from 'path';

export class UnsafePathError extends Error {}

const CONTROL = /[\u0000-\u001f\u007f]/;
const LOCALE = /^[A-Za-z]{2,3}(-[A-Za-z0-9]{2,8})*$/;
const WORKSPACE = /^[A-Za-z0-9_][A-Za-z0-9_.:-]*$/;

function decodeAll(s: string): string {
  let prev = s;
  for (let i = 0; i < 3; i++) {
    try {
      const next = decodeURIComponent(prev);
      if (next === prev) return next;
      prev = next;
    } catch {
      return prev;
    }
  }
  return prev;
}

/** Why `segment` is not a safe file name, or null when it is. */
export function segmentProblem(segment: string): string | null {
  if (segment === '') return 'empty path segment';
  if (CONTROL.test(segment)) return 'control character in name';
  if (segment.includes('/') || segment.includes('\\')) return 'path separator in name';
  if (segment === '.' || segment === '..') return 'dot segment';
  const decoded = decodeAll(segment);
  if (decoded === '.' || decoded === '..' || /[/\\]/.test(decoded) || CONTROL.test(decoded)) {
    return 'encoded dot segment or separator';
  }
  if (/^[A-Za-z]:/.test(segment)) return 'drive letter';
  if (segment.includes(':')) return 'colon in name';
  if (/[. ]$/.test(segment)) return 'trailing dot or space';
  if (segment.length > 255) return 'name longer than 255 characters';
  return null;
}

export function isSafeLocale(locale: string): boolean {
  return LOCALE.test(locale);
}

export function isSafeWorkspace(workspace: string): boolean {
  return WORKSPACE.test(workspace) && !workspace.includes('..');
}

/**
 * Check a server node path ("/a/b") segment by segment. Returns the segments,
 * or throws UnsafePathError naming the problem.
 */
export function safeNodeSegments(nodePath: string): string[] {
  if (!nodePath.startsWith('/')) throw new UnsafePathError(`node path is not absolute: ${JSON.stringify(nodePath)}`);
  const segments = nodePath.slice(1).split('/');
  for (const s of segments) {
    const problem = segmentProblem(s);
    if (problem) throw new UnsafePathError(`${problem}: ${JSON.stringify(nodePath)}`);
  }
  return segments;
}

/**
 * Resolve `rel` (posix, content-relative) inside `base` for writing or
 * deleting. Throws UnsafePathError when it would leave the base, or when an
 * existing component on the way — including the target — is a symlink.
 */
export function resolveInside(base: string, rel: string): string {
  if (path.isAbsolute(rel) || /^[A-Za-z]:/.test(rel)) throw new UnsafePathError(`absolute path: ${rel}`);
  const segments = rel.split('/');
  for (const s of segments) {
    const problem = segmentProblem(s);
    if (problem) throw new UnsafePathError(`${problem}: ${JSON.stringify(rel)}`);
  }
  const root = fs.realpathSync(base);
  const target = path.resolve(root, ...segments);
  if (!target.startsWith(root + path.sep)) throw new UnsafePathError(`outside the package: ${rel}`);

  let current = root;
  for (const s of segments) {
    current = path.join(current, s);
    let st: fs.Stats;
    try {
      st = fs.lstatSync(current);
    } catch {
      break; // does not exist yet: nothing below it can be a link either
    }
    if (st.isSymbolicLink()) throw new UnsafePathError(`refusing to write through a symlink: ${rel}`);
  }
  return target;
}
