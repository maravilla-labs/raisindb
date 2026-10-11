/**
 * Decide, per file, what a pull does — and then do it.
 *
 * Deciding never writes; applying only writes what was decided. `--dry-run`
 * is therefore exactly the decision list, not an approximation of it.
 */

import fs from 'fs';
import path from 'path';
import yaml from 'yaml';
import { skillMdToNodeYaml } from '../skill-md.js';
import type { DesiredBinary, DesiredFile, DesiredText } from './layout.js';
import { equalModuloEnv, overlaysEqual, sha256, sortKeysDeep, type JsonObject } from './normalize.js';
import type { PullClient } from './remote.js';
import { resolveInside } from './safe-path.js';
import type { PullState } from './state.js';
import { mergeNodeYaml, mergeYaml, nodeFileObject, nodeToSkillMd, renderYaml, type NodeFileValue } from './yaml-merge.js';

export type Action =
  | 'create' // not on disk yet
  | 'update' // server changed, local did not
  | 'unchanged'
  | 'local-edit' // local changed, server did not — kept, needs a push
  | 'conflict' // both changed (or no baseline and they differ)
  | 'deleted-locally' // pull wrote it before, someone deleted it since — kept deleted
  | 'verify'; // binary with no server hash: download decides

export interface PlannedFile {
  file: DesiredFile;
  action: Action;
  /** sha256 of the local bytes, when the file exists */
  localHash?: string;
  localMtime?: Date;
  /** A binary already downloaded to a temp file while verifying it */
  tmp?: string;
  tmpHash?: string;
}

/** A local file whose node is in scope but no longer on the server. */
export interface Orphan {
  rel: string;
  key: string;
}

export interface PlanContext {
  contentBase: string;
  state: PullState;
}

function readLocal(full: string): { bytes: Buffer; mtime: Date } | null {
  try {
    const st = fs.statSync(full);
    if (!st.isFile()) return null;
    return { bytes: fs.readFileSync(full), mtime: st.mtime };
  } catch {
    return null;
  }
}

/** Does the local text already mean what the server says? */
export function textMatches(file: DesiredText, text: string): boolean {
  try {
    if (file.kind === 'skill') {
      // Trailing whitespace of a Markdown body means nothing, and the
      // installer does not keep it byte for byte.
      const trimBody = (p: unknown): JsonObject => {
        const o = { ...((p ?? {}) as JsonObject) };
        if (typeof o.body === 'string') o.body = o.body.trimEnd();
        return o;
      };
      const local = yaml.parse(skillMdToNodeYaml(text)) as JsonObject;
      return equalModuloEnv(trimBody(local.properties), trimBody(file.value.properties));
    }
    const local = (yaml.parse(text) ?? {}) as JsonObject;
    if (file.kind === 'translation') return overlaysEqual(local, file.value);
    if (file.kind === 'asset-meta') return equalModuloEnv(local, file.value);
    return (
      local.node_type === file.value.node_type &&
      (local.archetype ?? null) === (file.value.archetype ?? null) &&
      equalModuloEnv(local.properties ?? {}, file.value.properties)
    );
  } catch {
    // An unparseable local file never "matches" — it is a local edit.
    return false;
  }
}

function binaryMatches(file: DesiredBinary, bytes: Buffer, localHash: string): boolean | null {
  if (file.inlineCode !== null) return sha256(file.inlineCode) === localHash;
  if (file.contentHash) return file.contentHash === localHash;
  return null; // unknown without the bytes
}

export function classify(files: DesiredFile[], ctx: PlanContext): PlannedFile[] {
  return files.map((file) => {
    const local = readLocal(resolveInside(ctx.contentBase, file.rel));
    const base = ctx.state.get(file.rel);
    if (!local) return { file, action: base ? 'deleted-locally' : 'create' };

    const localHash = sha256(local.bytes);
    const out = { file, localHash, localMtime: local.mtime };
    const matches =
      file.kind === 'binary'
        ? binaryMatches(file, local.bytes, localHash)
        : textMatches(file, local.bytes.toString('utf-8'));

    if (matches === true) return { ...out, action: 'unchanged' };
    if (matches === null) {
      // No server hash to compare with: the baseline decides when it can,
      // and only otherwise is the file downloaded to find out.
      const sameServer = !!base && base.server === file.serverHash;
      const sameLocal = !!base && base.local === localHash;
      if (sameServer && sameLocal) return { ...out, action: 'unchanged' };
      if (sameLocal) return { ...out, action: 'update' };
      if (sameServer) return { ...out, action: 'local-edit' };
      return { ...out, action: 'verify' };
    }
    const localChanged = !base || base.local !== localHash;
    const serverChanged = !base || base.server !== file.serverHash;
    if (!localChanged) return { ...out, action: 'update' };
    if (!serverChanged) return { ...out, action: 'local-edit' };
    return { ...out, action: 'conflict' };
  });
}

/** Local files under the scanned roots that the server no longer has. */
export function findOrphans(
  indexed: Map<string, string>,
  desiredRels: Set<string>,
  inScope: (key: string, rel: string) => boolean
): Orphan[] {
  const orphans: Orphan[] = [];
  for (const [rel, key] of indexed) {
    if (desiredRels.has(rel)) continue;
    if (inScope(key, rel)) orphans.push({ rel, key });
  }
  return orphans.sort((a, b) => a.rel.localeCompare(b.rel));
}

// ---------------------------------------------------------------------------
// Applying
// ---------------------------------------------------------------------------

/**
 * A temp path next to `full` that is guaranteed to be a NEW regular file: an
 * existing entry of that name (a planted symlink included) is removed first,
 * and the file is then created exclusively.
 */
export function freshTemp(full: string): string {
  const tmp = `${full}.raisindb-partial`;
  fs.rmSync(tmp, { force: true });
  return tmp;
}

function atomicWrite(full: string, data: string | Buffer): void {
  fs.mkdirSync(path.dirname(full), { recursive: true });
  const tmp = freshTemp(full);
  fs.writeFileSync(tmp, data, { flag: 'wx' });
  fs.renameSync(tmp, full);
}

/** The text a desired file should have, merged into what is on disk. */
export function renderText(file: DesiredText, existing: string | null): string {
  if (file.kind === 'skill') return nodeToSkillMd(file.value.properties as JsonObject);
  if (file.kind === 'node') {
    const v = file.value as unknown as NodeFileValue;
    return existing !== null ? mergeNodeYaml(existing, v) : renderYaml(nodeFileObject(v));
  }
  return existing !== null ? mergeYaml(existing, file.value) : renderYaml(sortKeysDeep(file.value) as JsonObject);
}

export interface ApplyOutcome {
  written: boolean;
  error?: string;
}

async function downloadVerified(
  file: DesiredBinary,
  full: string,
  client: PullClient
): Promise<{ tmp: string; sha256: string }> {
  fs.mkdirSync(path.dirname(full), { recursive: true });
  const tmp = freshTemp(full);
  const got = await client.download(file.workspace, file.nodePath, tmp);
  if (file.contentHash && got.sha256 !== file.contentHash) {
    fs.rmSync(tmp, { force: true });
    throw new Error(`downloaded bytes do not match the server's content_hash (${got.size} bytes)`);
  }
  return { tmp, sha256: got.sha256 };
}

/**
 * For a `verify` binary: download it and compare. Identical → it becomes
 * `unchanged` (baseline recorded). Different → it becomes a `conflict` whose
 * download is kept in a temp file until the conflict is resolved.
 */
export async function verifyBinary(
  planned: PlannedFile,
  ctx: PlanContext & { client: PullClient }
): Promise<PlannedFile> {
  const file = planned.file as DesiredBinary;
  const full = resolveInside(ctx.contentBase, file.rel);
  const got = await downloadVerified(file, full, ctx.client);
  if (got.sha256 === planned.localHash) {
    fs.rmSync(got.tmp, { force: true });
    ctx.state.set(file.rel, { server: file.serverHash, local: got.sha256 });
    return { ...planned, action: 'unchanged' };
  }
  return { ...planned, action: 'conflict', tmp: got.tmp, tmpHash: got.sha256 };
}

/** Drop a verify download that was not taken. */
export function discardTemp(planned: PlannedFile): void {
  if (planned.tmp) fs.rmSync(planned.tmp, { force: true });
}

/**
 * Write one file. A binary is streamed to a temp file first and only renamed
 * into place after its hash checks out, so a cut connection never leaves a
 * truncated image where a good one was.
 */
export async function applyFile(
  planned: PlannedFile,
  ctx: PlanContext & { client: PullClient }
): Promise<ApplyOutcome> {
  const { file } = planned;
  try {
    const full = resolveInside(ctx.contentBase, file.rel);
    if (file.kind !== 'binary') {
      const existing = fs.existsSync(full) ? fs.readFileSync(full, 'utf-8') : null;
      const text = renderText(file, existing);
      atomicWrite(full, text);
      ctx.state.set(file.rel, { server: file.serverHash, local: sha256(text) });
      return { written: true };
    }

    if (file.inlineCode !== null) {
      atomicWrite(full, file.inlineCode);
      ctx.state.set(file.rel, { server: file.serverHash, local: sha256(file.inlineCode) });
      return { written: true };
    }

    const got = planned.tmp
      ? { tmp: planned.tmp, sha256: planned.tmpHash as string }
      : await downloadVerified(file, full, ctx.client);
    fs.renameSync(got.tmp, full);
    ctx.state.set(file.rel, { server: file.serverHash, local: got.sha256 });
    return { written: true };
  } catch (err) {
    return { written: false, error: err instanceof Error ? err.message : String(err) };
  }
}

/** The text a conflict would be resolved to, for `--diff`. */
export function proposedText(file: DesiredText, contentBase: string): { before: string; after: string } {
  const full = resolveInside(contentBase, file.rel);
  const before = fs.existsSync(full) ? fs.readFileSync(full, 'utf-8') : '';
  return { before, after: renderText(file, before || null) };
}
