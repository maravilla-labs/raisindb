/**
 * What a pull tells the person running it: progress while it works, then a
 * summary that names every file it touched or refused to touch.
 */

import type { Action, Orphan, PlannedFile } from './plan.js';

/** One updating line on a terminal; plain lines in a log. */
export class Progress {
  private last = '';
  private lastPrint = 0;
  constructor(private tty = !!process.stderr.isTTY, private out: NodeJS.WriteStream = process.stderr) {}

  update(line: string): void {
    if (this.tty) {
      this.out.write(`\r\x1b[2K${line}`);
      this.last = line;
      return;
    }
    // Logs: at most one line every 2s, so a 10k-node pull does not flood CI.
    const now = Date.now();
    if (now - this.lastPrint > 2000) {
      this.out.write(`${line}\n`);
      this.lastPrint = now;
    }
  }

  done(line: string): void {
    if (this.tty && this.last) this.out.write('\r\x1b[2K');
    this.last = '';
    this.out.write(`${line}\n`);
  }
}

export const ACTION_LABELS: Record<Action | 'orphan' | 'owned' | 'policy' | 'ignored', string> = {
  create: 'new',
  update: 'updated',
  unchanged: 'unchanged',
  'local-edit': 'local edits kept',
  conflict: 'conflicts',
  'deleted-locally': 'deleted locally, kept deleted',
  verify: 'to compare (no server hash)',
  orphan: 'gone from server',
  owned: 'package-owned, skipped',
  policy: 'excluded by .raisin-sync.yaml',
  ignored: 'ignored',
};

const MARK: Partial<Record<Action, string>> = {
  create: '+',
  update: '~',
  'local-edit': 'L',
  conflict: '!',
  'deleted-locally': 'x',
  verify: '?',
};

export function listFiles(
  title: string,
  items: string[],
  mark: string,
  limit: number,
  log: (s: string) => void
): void {
  if (items.length === 0) return;
  log(`\n${title} (${items.length}):`);
  for (const item of items.slice(0, limit)) log(`  ${mark} ${item}`);
  if (items.length > limit) log(`  … and ${items.length - limit} more (--verbose lists all)`);
}

export function printPlan(
  planned: PlannedFile[],
  orphans: Orphan[],
  opts: { verbose: boolean; log: (s: string) => void }
): void {
  const limit = opts.verbose ? Infinity : 40;
  const by = (a: Action) => planned.filter((p) => p.action === a).map((p) => p.file.rel).sort();
  for (const action of ['create', 'update', 'verify', 'local-edit', 'conflict', 'deleted-locally'] as Action[]) {
    listFiles(ACTION_LABELS[action], by(action), MARK[action] ?? ' ', limit, opts.log);
  }
  listFiles(ACTION_LABELS.orphan, orphans.map((o) => o.rel), '-', limit, opts.log);
}

// ---------------------------------------------------------------------------
// Unified diff (for --diff)
// ---------------------------------------------------------------------------

const MAX_DIFF_LINES = 3000;

/** A unified diff of two texts, three lines of context. */
export function unifiedDiff(before: string, after: string, label: string): string {
  const a = before.split('\n');
  const b = after.split('\n');
  if (a.length > MAX_DIFF_LINES || b.length > MAX_DIFF_LINES) {
    return `--- local/${label}\n+++ server/${label}\n(file too large to diff: ${a.length} → ${b.length} lines)`;
  }
  // LCS table, then walk it into an edit script.
  const n = a.length;
  const m = b.length;
  const lcs: Uint32Array[] = Array.from({ length: n + 1 }, () => new Uint32Array(m + 1));
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      lcs[i][j] = a[i] === b[j] ? lcs[i + 1][j + 1] + 1 : Math.max(lcs[i + 1][j], lcs[i][j + 1]);
    }
  }
  const ops: Array<{ t: ' ' | '-' | '+'; s: string }> = [];
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (a[i] === b[j]) {
      ops.push({ t: ' ', s: a[i++] });
      j++;
    } else if (lcs[i + 1][j] >= lcs[i][j + 1]) ops.push({ t: '-', s: a[i++] });
    else ops.push({ t: '+', s: b[j++] });
  }
  while (i < n) ops.push({ t: '-', s: a[i++] });
  while (j < m) ops.push({ t: '+', s: b[j++] });

  const out = [`--- local/${label}`, `+++ server/${label}`];
  const CONTEXT = 3;
  let k = 0;
  while (k < ops.length) {
    if (ops[k].t === ' ') {
      k++;
      continue;
    }
    const start = Math.max(0, k - CONTEXT);
    let end = k;
    // Extend the hunk while changes are within 2*CONTEXT of each other.
    while (end < ops.length) {
      if (ops[end].t !== ' ') {
        end++;
        continue;
      }
      let run = 0;
      while (end + run < ops.length && ops[end + run].t === ' ') run++;
      if (end + run >= ops.length || run > CONTEXT * 2) {
        end = Math.min(ops.length, end + CONTEXT);
        break;
      }
      end += run;
    }
    out.push('@@');
    for (const op of ops.slice(start, end)) out.push(`${op.t}${op.s}`);
    k = end;
  }
  return out.join('\n');
}
