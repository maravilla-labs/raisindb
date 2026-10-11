/**
 * `raisindb pull` / `raisindb sync --pull`: bring a package folder up to date
 * with what a server holds.
 *
 *   scan    every root, full depth, paged; plus each locale for translations
 *   layout  nodes → the files `package create` / `deploy` consume
 *   decide  per file: new / updated / unchanged / local edit / conflict
 *   apply   write (atomically; binaries streamed), record baselines
 *
 * A read that comes back EMPTY is the dangerous case: RaisinDB answers an
 * unauthenticated or unauthorised read with zero rows, not 401. So an empty
 * root never turns into "delete the local files", and a pull where every root
 * is empty stops and says why instead of reporting success.
 */

import fs from 'fs';
import path from 'path';
import type { SyncConfig } from '../config.js';
import { mapChangeToNode } from '../mapping.js';
import { layoutNodes, type DesiredFile } from './layout.js';
import { cleanProperties, computeOverlay, type JsonObject } from './normalize.js';
import {
  applyFile,
  classify,
  discardTemp,
  findOrphans,
  proposedText,
  verifyBinary,
  type Orphan,
  type PlannedFile,
} from './plan.js';
import { countSubtree, listTopLevel, scanSubtree, type PullClient, type RemoteNode } from './remote.js';
import { ACTION_LABELS, listFiles, printPlan, Progress, unifiedDiff } from './report.js';
import {
  buildLocalIndex,
  defaultRoots,
  formatRoot,
  isUnder,
  loadInstallPolicy,
  nodeKey,
  parseRootSpec,
  policyFor,
  type Root,
} from './scope.js';
import { PullState } from './state.js';

/** Above this many roots in one workspace, scan the workspace once and filter. */
const WHOLE_WORKSPACE_THRESHOLD = 8;
/** Concurrent binary downloads. */
const DOWNLOAD_CONCURRENCY = 4;

export type ConflictChoice = 'server' | 'local';

export interface PullOptions {
  packageDir: string;
  config: SyncConfig;
  client: PullClient;
  /** Content-relative path → excluded by the ignore list / .rapignore */
  isIgnored?: (rel: string) => boolean;
  /** `ws:/path` specs replacing the default roots */
  paths?: string[];
  includeOwned?: boolean;
  /** Apply deletions of files whose node is gone from the server */
  delete?: boolean;
  dryRun?: boolean;
  force?: boolean;
  diff?: boolean;
  verbose?: boolean;
  translations?: boolean;
  allowEmpty?: boolean;
  /** SQL page size (tests use a small one to cross page boundaries) */
  pageSize?: number;
  /** Ask about a conflict (TTY with conflict_strategy: prompt). */
  askConflict?: (file: PlannedFile, showDiff: () => string) => Promise<ConflictChoice | 'server-all' | 'local-all'>;
  log?: (s: string) => void;
  progress?: Progress;
}

export interface PullReport {
  counts: Record<string, number>;
  serverNodes: number;
  filesWritten: number;
  conflictsUnresolved: number;
  errors: string[];
  aborted?: string;
}

interface ScanResult {
  root: Root;
  nodes: RemoteNode[];
  serverCount: number | null;
  missingWorkspace?: boolean;
}

function contentBaseOf(packageDir: string): { contentBase: string; legacy: boolean } {
  const contentDir = path.join(packageDir, 'content');
  const isDir = fs.existsSync(contentDir) && fs.statSync(contentDir).isDirectory();
  return { contentBase: isDir ? contentDir : packageDir, legacy: !isDir };
}

function isMissingWorkspace(err: unknown): boolean {
  const msg = err instanceof Error ? err.message : String(err);
  // The SQL layer names a workspace a "table".
  return /(workspace|table)/i.test(msg) && /(not found|does not exist|unknown)/i.test(msg);
}

export async function runPull(opts: PullOptions): Promise<PullReport> {
  const log = opts.log ?? ((s: string) => console.log(s));
  const progress = opts.progress ?? new Progress();
  const { config, client } = opts;
  const { contentBase, legacy } = contentBaseOf(opts.packageDir);
  const isIgnored = opts.isIgnored ?? (() => false);
  const report: PullReport = { counts: {}, serverNodes: 0, filesWritten: 0, conflictsUnresolved: 0, errors: [] };
  const bump = (k: string, n = 1) => (report.counts[k] = (report.counts[k] ?? 0) + n);

  // ---- scope --------------------------------------------------------------
  const index = buildLocalIndex(contentBase, isIgnored, legacy);
  const explicit = opts.paths?.length ? opts.paths : config.pull?.roots;
  const roots: Root[] = explicit?.length ? explicit.map(parseRootSpec) : defaultRoots(index);
  const excludes = (config.pull?.exclude ?? []).map(parseRootSpec);
  if (roots.length === 0) {
    report.aborted = 'Nothing to pull: the package has no content and no roots were given (use --path ws:/path).';
    return report;
  }
  const policy = loadInstallPolicy(opts.packageDir);
  log(`Pulling ${roots.length} root(s)${explicit?.length ? '' : ' (what the package contains)'}:`);
  for (const r of roots.slice(0, opts.verbose ? Infinity : 12)) log(`  ${formatRoot(r)}`);
  if (!opts.verbose && roots.length > 12) log(`  … and ${roots.length - 12} more`);

  // ---- locales ------------------------------------------------------------
  let locales: string[] = [];
  if (opts.translations !== false) {
    const info = await client.repoLocales().catch(() => null);
    if (info) locales = info.supported.filter((l) => l !== info.defaultLanguage);
  }

  // ---- scan ---------------------------------------------------------------
  const byWorkspace = new Map<string, Root[]>();
  for (const r of roots) {
    if (!byWorkspace.has(r.workspace)) byWorkspace.set(r.workspace, []);
    byWorkspace.get(r.workspace)!.push(r);
  }

  const scans: ScanResult[] = [];
  const localized = new Map<string, Map<string, RemoteNode>>(); // locale → key → node
  for (const locale of locales) localized.set(locale, new Map());

  for (const [workspace, wsRoots] of byWorkspace) {
    const whole = wsRoots.some((r) => r.path === '/') || wsRoots.length > WHOLE_WORKSPACE_THRESHOLD;
    const scanRoots: Root[] = whole ? [{ workspace, path: '/' }] : wsRoots;
    let wsNodes = 0;
    let missing = false;
    for (const scanRoot of scanRoots) {
      const label = formatRoot(scanRoot);
      try {
        const nodes = await scanSubtree(client, workspace, scanRoot.path, {
          pageSize: opts.pageSize,
          onPage: (n) => progress.update(`Reading ${label} … ${n} nodes`),
        });
        const serverCount = await countSubtree(client, workspace, scanRoot.path).catch(() => null);
        for (const locale of locales) {
          const locNodes = await scanSubtree(client, workspace, scanRoot.path, {
            locale,
            pageSize: opts.pageSize,
            onPage: (n) => progress.update(`Reading ${label} [${locale}] … ${n} nodes`),
          }).catch(() => []);
          for (const n of locNodes) localized.get(locale)!.set(nodeKey(workspace, n.path), n);
        }
        // A whole-workspace scan is split back into the roots it stands for.
        const targets = whole && !wsRoots.some((r) => r.path === '/') ? wsRoots : [scanRoot];
        for (const root of targets) {
          const inRoot = nodes.filter((n) => isUnder(n.path, root.path));
          scans.push({ root, nodes: inRoot, serverCount: targets.length === 1 ? serverCount : inRoot.length });
          wsNodes += inRoot.length;
        }
        if (serverCount !== null && serverCount !== nodes.length) {
          log(`  ! ${label}: the server reports ${serverCount} nodes, ${nodes.length} were read — the tree changed during the pull, or the read was cut short. Rerun to settle it.`);
        }
      } catch (err) {
        if (isMissingWorkspace(err)) {
          missing = true;
          for (const root of whole ? wsRoots : scanRoots) scans.push({ root, nodes: [], serverCount: 0, missingWorkspace: true });
          break;
        }
        throw err;
      }
    }
    const what = wsRoots.length === 1 ? formatRoot(wsRoots[0]) : `${workspace} (${wsRoots.length} roots)`;
    progress.done(missing ? `! ${what}: workspace does not exist on the server` : `Read ${what}: ${wsNodes} node(s)`);
  }

  // ---- the empty-read guard -----------------------------------------------
  const localUnder = (r: Root) =>
    [...index.fileNode.values()].filter((k) => {
      const [ws, p] = k.split('\u0000');
      return ws === r.workspace && isUnder(p, r.path);
    }).length;
  const emptyWithLocal = scans
    .filter((s) => s.nodes.length === 0)
    .map((s) => ({ root: s.root, local: localUnder(s.root) }))
    .filter((e) => e.local > 0);
  if (emptyWithLocal.length > 0) {
    log(`\n${emptyWithLocal.length} root(s) came back EMPTY although the package has files there — nothing under them is treated as deleted:`);
    for (const e of emptyWithLocal.slice(0, opts.verbose ? Infinity : 8)) {
      log(`  ! ${formatRoot(e.root)} (${e.local} local file(s))`);
    }
    if (!opts.verbose && emptyWithLocal.length > 8) log(`  … and ${emptyWithLocal.length - 8} more`);
  }
  report.serverNodes = scans.reduce((n, s) => n + s.nodes.length, 0);
  if (report.serverNodes === 0 && !opts.allowEmpty) {
    report.aborted =
      'The server returned no nodes for any root. RaisinDB answers a read without valid credentials with ' +
      'an EMPTY result, not an error — so this usually means the token is missing, expired, or for another ' +
      'server/repository. Check with `raisindb repo list`, log in to this server (or set RAISINDB_TOKEN), ' +
      'and check --repo. If the server really is empty, rerun with --allow-empty.';
    return report;
  }

  // ---- filter by policy / excludes, then lay out --------------------------
  const pulled: RemoteNode[] = [];
  const policySkipped: string[] = [];
  const ownedSkipped: string[] = [];
  const seenNodes = new Set<string>();
  for (const s of scans) {
    for (const n of s.nodes) {
      // Overlapping --path roots must not lay the same node out twice.
      const k = nodeKey(n.workspace, n.path);
      if (seenNodes.has(k)) continue;
      seenNodes.add(k);
      if (excludes.some((e) => e.workspace === n.workspace && isUnder(n.path, e.path))) {
        policySkipped.push(`${n.workspace}:${n.path}`);
        continue;
      }
      const verdict = policyFor(policy, n.workspace, n.path);
      if (!verdict.pull) {
        policySkipped.push(`${n.workspace}:${n.path}`);
        continue;
      }
      if (verdict.owned && !opts.includeOwned) {
        ownedSkipped.push(`${n.workspace}:${n.path}`);
        continue;
      }
      pulled.push(n);
    }
  }

  const overlays = new Map<string, Map<string, JsonObject>>();
  if (locales.length > 0) {
    for (const n of pulled) {
      const key = nodeKey(n.workspace, n.path);
      const base = cleanProperties(n.properties);
      for (const locale of locales) {
        const loc = localized.get(locale)!;
        if (loc.size === 0) continue; // locale view unavailable — not "hidden"
        const ln = loc.get(key);
        const overlay = ln ? computeOverlay(base, cleanProperties(ln.properties)) : { hidden: true };
        if (Object.keys(overlay).length === 0) continue;
        if (!overlays.has(key)) overlays.set(key, new Map());
        overlays.get(key)!.set(locale, overlay);
      }
    }
  }

  const layout = layoutNodes(pulled, index, overlays);
  const desired: DesiredFile[] = [];
  let ignored = 0;
  for (const f of layout.files) {
    if (isIgnored(f.rel)) ignored++;
    else desired.push(f);
  }

  // ---- decide ---------------------------------------------------------------
  const state = PullState.load(opts.packageDir, PullState.targetKey(config.server, config.repository, config.branch));
  let planned = classify(desired, { contentBase, state });

  const desiredRels = new Set(desired.map((f) => f.rel));
  const scannedOk = scans.filter((s) => s.nodes.length > 0).map((s) => s.root);
  const orphans: Orphan[] = findOrphans(index.fileNode, desiredRels, (key, rel) => {
    const [ws, p] = key.split('\u0000');
    if (!scannedOk.some((r) => r.workspace === ws && isUnder(p, r.path))) return false;
    if (excludes.some((e) => e.workspace === ws && isUnder(p, e.path))) return false;
    const verdict = policyFor(policy, ws, p);
    if (!verdict.pull || (verdict.owned && !opts.includeOwned)) return false;
    // A translation for a locale this pull did not read is not "gone".
    const locale = mapChangeToNode(rel).locale;
    if (locale && !locales.includes(locale)) return false;
    return true;
  });

  // ---- resolve conflicts ------------------------------------------------------
  // `verify` binaries are downloaded first: only the bytes can say whether
  // they differ. (In a dry run they are reported, not downloaded.)
  if (!opts.dryRun) {
    const verify = planned.filter((p) => p.action === 'verify');
    let done = 0;
    for (const p of verify) {
      progress.update(`Comparing binaries without a server hash … ${++done}/${verify.length}`);
      try {
        planned[planned.indexOf(p)] = await verifyBinary(p, { contentBase, state, client });
      } catch (err) {
        report.errors.push(`${p.file.rel}: ${err instanceof Error ? err.message : String(err)}`);
        planned[planned.indexOf(p)] = { ...p, action: 'local-edit' };
      }
    }
    if (verify.length) progress.done(`Compared ${verify.length} binar${verify.length === 1 ? 'y' : 'ies'}`);
  }

  const strategy = opts.force ? 'server' : config.conflict_strategy ?? 'prompt';
  const take = new Set<PlannedFile>();
  let all: ConflictChoice | null = null;
  for (const p of planned.filter((x) => x.action === 'conflict')) {
    let choice: ConflictChoice | null = null;
    if (strategy === 'server') choice = 'server';
    else if (strategy === 'local') choice = 'local';
    else if (strategy === 'newest') {
      const serverTime = p.file.updatedAt ? Date.parse(p.file.updatedAt) : 0;
      choice = serverTime > (p.localMtime?.getTime() ?? 0) ? 'server' : 'local';
    } else if (all) choice = all;
    else if (opts.askConflict && !opts.dryRun) {
      const answer = await opts.askConflict(p, () => diffFor(p, contentBase));
      if (answer === 'server-all' || answer === 'local-all') {
        all = answer === 'server-all' ? 'server' : 'local';
        choice = all;
      } else choice = answer;
    }
    if (choice === 'server') take.add(p);
  }
  // `--force` also restores files deleted locally since the last pull.
  if (opts.force) for (const p of planned) if (p.action === 'deleted-locally') take.add(p);

  // ---- report the plan ------------------------------------------------------
  const counts = (a: string) => planned.filter((p) => p.action === a).length;
  for (const a of ['create', 'update', 'unchanged', 'local-edit', 'deleted-locally', 'verify']) {
    bump(a, counts(a));
  }
  const conflicts = planned.filter((p) => p.action === 'conflict');
  bump('conflict', conflicts.filter((p) => !take.has(p)).length);
  bump('conflict-server', conflicts.filter((p) => take.has(p)).length);
  bump('orphan', orphans.length);
  bump('owned', ownedSkipped.length);
  bump('policy', policySkipped.length);
  bump('ignored', ignored);
  bump('derived', layout.derivedSkipped);

  printPlan(planned, orphans, { verbose: !!opts.verbose, log });
  if (ownedSkipped.length) {
    log(`\n${ownedSkipped.length} node(s) skipped as package-owned (mode: replace in .raisin-sync.yaml — a deploy overwrites them, so the package is their source). --include-owned pulls them too.`);
  }
  if (policySkipped.length) listFiles(ACTION_LABELS.policy, policySkipped, ' ', opts.verbose ? Infinity : 10, log);
  if (layout.derivedSkipped) log(`\n${layout.derivedSkipped} node(s) below assets skipped (renditions/thumbnails the server derives).`);
  if (opts.diff) {
    for (const p of planned.filter((x) => x.action === 'update' || x.action === 'conflict')) {
      const d = diffFor(p, contentBase);
      if (d) log(`\n${d}`);
    }
  }
  if (!explicit?.length) await reportOutside(client, byWorkspace, log).catch(() => undefined);

  // ---- apply ------------------------------------------------------------------
  const toWrite = planned.filter((p) => p.action === 'create' || p.action === 'update' || take.has(p));
  report.conflictsUnresolved = planned.filter((p) => p.action === 'conflict' && !take.has(p)).length;
  if (opts.dryRun) {
    for (const p of planned) discardTemp(p);
    log(`\n[dry run] ${summaryLine(report.counts)} — nothing written.`);
    return report;
  }

  let written = 0;
  const queue = [...toWrite];
  const worker = async () => {
    for (let p = queue.shift(); p; p = queue.shift()) {
      const out = await applyFile(p, { contentBase, state, client });
      if (out.error) report.errors.push(`${p.file.rel}: ${out.error}`);
      if (out.written) written++;
      progress.update(`Writing … ${written}/${toWrite.length}`);
      if (written % 50 === 0) state.save();
    }
  };
  await Promise.all(Array.from({ length: DOWNLOAD_CONCURRENCY }, worker));
  for (const p of planned) if (!take.has(p)) discardTemp(p);
  // Files that already matched get a baseline, so the next pull is three-way.
  for (const p of planned) {
    if (p.action === 'unchanged' && p.localHash) state.set(p.file.rel, { server: p.file.serverHash, local: p.localHash });
  }

  if (opts.delete) {
    for (const o of orphans) {
      fs.rmSync(path.join(contentBase, o.rel), { force: true });
      state.delete(o.rel);
    }
  }
  state.save();
  if (toWrite.length) progress.done(`Wrote ${written} file(s)`);
  report.filesWritten = written;

  log(`\n${summaryLine(report.counts)}`);
  if (orphans.length && !opts.delete) log(`${orphans.length} local file(s) are gone from the server; --delete removes them.`);
  if (report.conflictsUnresolved) {
    log(`${report.conflictsUnresolved} conflict(s) left as they are. --force takes the server version; --diff shows the differences.`);
  }
  for (const e of report.errors) log(`  ✗ ${e}`);
  return report;
}

function diffFor(p: PlannedFile, contentBase: string): string {
  if (p.file.kind === 'binary') return `${p.file.rel}: binary differs`;
  const { before, after } = proposedText(p.file, contentBase);
  return unifiedDiff(before, after, p.file.rel);
}

function summaryLine(c: Record<string, number>): string {
  const parts = [
    ['new', c.create], ['updated', c.update], ['unchanged', c.unchanged], ['local edits', c['local-edit']],
    ['conflicts taken from the server', c['conflict-server']], ['conflicts left', c.conflict],
    ['gone from server', c.orphan], ['to compare', c.verify],
  ].filter(([, n]) => n) as Array<[string, number]>;
  return parts.length ? parts.map(([k, n]) => `${n} ${k}`).join(', ') : 'nothing to do';
}

/** Top-level server nodes in a scanned workspace that no root covers. */
async function reportOutside(client: PullClient, byWorkspace: Map<string, Root[]>, log: (s: string) => void) {
  for (const [workspace, roots] of byWorkspace) {
    if (roots.some((r) => r.path === '/')) continue;
    const top = await listTopLevel(client, workspace);
    const outside = top.filter((p) => !roots.some((r) => isUnder(p, r.path) || isUnder(r.path, p)));
    if (outside.length === 0) continue;
    const shown = outside.slice(0, 5).map((p) => `${workspace}:${p}`).join(', ');
    log(`\nNot part of the package, so not pulled — ${workspace}: ${shown}${outside.length > 5 ? ` (+${outside.length - 5})` : ''}`);
    log(`  Add one with --path ${workspace}:${outside[0]} (or the whole workspace with --path ${workspace}:/), or list it under pull.roots in .raisindb-cli.yaml.`);
  }
}
