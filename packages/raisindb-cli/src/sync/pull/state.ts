/**
 * What the last pull wrote, so the next one can tell WHO changed a file.
 *
 * Without a baseline a difference between a local file and the server is
 * ambiguous — an editor changed the page on the live site, or a developer
 * changed the file and has not deployed yet — and the only safe answer is
 * "conflict". With it, pull does a three-way decision per file:
 *
 *   local unchanged since last pull, server changed → update the file
 *   local changed, server unchanged                 → keep the local edit
 *   both changed                                    → conflict
 *
 * Stored per target (server + repo + branch) in `.raisindb/pull-state.json`
 * inside the package, which `package create` and `sync --push` never ship.
 */

import fs from 'fs';
import path from 'path';
import { normalizeServerUrl } from '../../config.js';

export interface FileBaseline {
  /** serverHash of the version that was written (see layout.ts) */
  server: string;
  /** sha256 of the bytes on disk right after the write */
  local: string;
}

interface StateFile {
  version: 1;
  targets: Record<string, { pulled_at?: string; files: Record<string, FileBaseline> }>;
}

export const STATE_DIR = '.raisindb';
const STATE_FILE = 'pull-state.json';

export class PullState {
  private data: StateFile;
  private dirty = false;

  private constructor(private file: string, private target: string, data: StateFile) {
    this.data = data;
    if (!this.data.targets[target]) this.data.targets[target] = { files: {} };
  }

  static targetKey(server: string, repo: string, branch: string): string {
    return `${normalizeServerUrl(server)}|${repo}|${branch}`;
  }

  static load(packageDir: string, target: string): PullState {
    const file = path.join(packageDir, STATE_DIR, STATE_FILE);
    let data: StateFile = { version: 1, targets: {} };
    try {
      const parsed = JSON.parse(fs.readFileSync(file, 'utf-8')) as StateFile;
      if (parsed && parsed.version === 1 && parsed.targets) data = parsed;
    } catch {
      /* first pull, or an unreadable file: start without baselines */
    }
    return new PullState(file, target, data);
  }

  get(rel: string): FileBaseline | undefined {
    return this.data.targets[this.target].files[rel];
  }

  set(rel: string, baseline: FileBaseline): void {
    this.data.targets[this.target].files[rel] = baseline;
    this.dirty = true;
  }

  delete(rel: string): void {
    delete this.data.targets[this.target].files[rel];
    this.dirty = true;
  }

  /** Written after every batch, so an interrupted pull resumes from there. */
  save(): void {
    if (!this.dirty) return;
    this.data.targets[this.target].pulled_at = new Date().toISOString();
    fs.mkdirSync(path.dirname(this.file), { recursive: true });
    const tmp = `${this.file}.tmp`;
    fs.writeFileSync(tmp, JSON.stringify(this.data, null, 1));
    fs.renameSync(tmp, this.file);
    this.dirty = false;
  }
}
