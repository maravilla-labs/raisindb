import { apiCall, type FetchLike } from './admin-util.js';

/**
 * `raisindb repo gc` / `raisindb repo retention`: revision-history garbage
 * collection and the retention policy it applies.
 *
 * Every write keeps the previous version of the node and of each index row it
 * touches. GC removes versions older than the retention window — keeping HEAD,
 * every revision inside the window, and every tag and branch fork point —
 * deletes the upload blobs only those versions referenced, and compacts the
 * database so the space is returned to the disk.
 */

export interface GcOptions {
  tenant?: string;
  dryRun?: boolean;
  keepDays?: string;
  keepRevisions?: string;
  json?: boolean;
}

interface CfStats {
  versions_deleted: number;
  bytes_deleted: number;
}

export interface GcOutcome {
  dry_run: boolean;
  versions_deleted: number;
  bytes_deleted: number;
  oplog_entries_deleted: number;
  jobs_deleted: number;
  orphaned_blobs: string[];
  blobs_deleted: number;
  blob_bytes_deleted: number;
  live_sst_bytes_before: number;
  live_sst_bytes_after: number;
  duration_ms: number;
  column_families: Record<string, CfStats>;
}

export interface RetentionPolicy {
  keep_days?: number;
  keep_revisions?: number;
}

export interface RetentionOptions {
  tenant?: string;
  branch?: string;
  keepDays?: string;
  keepRevisions?: string;
  clear?: boolean;
  json?: boolean;
}

function parseCount(flag: string, value: string | undefined): number | undefined {
  if (value === undefined) return undefined;
  const n = Number(value);
  if (!Number.isInteger(n) || n < 0) {
    throw new Error(`${flag} must be a non-negative whole number, got '${value}'.`);
  }
  return n;
}

export function formatBytes(n: number): string {
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  let v = n;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(i === 0 ? 0 : 1)} ${units[i]}`;
}

function base(tenant: string | undefined, repo: string): string {
  return `/api/admin/management/database/${encodeURIComponent(tenant || 'default')}/${encodeURIComponent(repo)}/history`;
}

export async function repoGc(repo: string, options: GcOptions = {}, fetchImpl?: FetchLike): Promise<void> {
  const body: Record<string, unknown> = { dry_run: !!options.dryRun };
  const keepDays = parseCount('--keep-days', options.keepDays);
  const keepRevisions = parseCount('--keep-revisions', options.keepRevisions);
  if (keepDays !== undefined) body.keep_days = keepDays;
  if (keepRevisions !== undefined) body.keep_revisions = keepRevisions;

  const res = await apiCall<GcOutcome>(`${base(options.tenant, repo)}/gc`, { method: 'POST', body, fetchImpl });
  if (!res.ok || !res.data) {
    throw new Error(`History GC of '${repo}' failed: ${res.errorMessage}`);
  }
  const o = res.data;
  if (options.json) {
    console.log(JSON.stringify(o, null, 2));
    return;
  }
  const verb = o.dry_run ? 'Would remove' : 'Removed';
  console.log(`${verb} ${o.versions_deleted} superseded versions (${formatBytes(o.bytes_deleted)} of data) from '${repo}'.`);
  const touched = Object.entries(o.column_families)
    .filter(([, s]) => s.versions_deleted > 0)
    .sort((a, b) => b[1].bytes_deleted - a[1].bytes_deleted);
  for (const [cf, s] of touched) {
    console.log(`  ${cf.padEnd(18)} ${String(s.versions_deleted).padStart(9)} versions  ${formatBytes(s.bytes_deleted)}`);
  }
  if (o.oplog_entries_deleted > 0) console.log(`  operation log       ${o.oplog_entries_deleted} entries`);
  if (o.jobs_deleted > 0) console.log(`  job history         ${o.jobs_deleted} finished jobs`);
  if (o.dry_run) {
    console.log(`Blobs only old versions reference: ${o.orphaned_blobs.length}`);
    console.log('Dry run: nothing was changed.');
  } else {
    console.log(`Deleted ${o.blobs_deleted} orphaned blobs (${formatBytes(o.blob_bytes_deleted)}).`);
    console.log(
      `Database files: ${formatBytes(o.live_sst_bytes_before)} -> ${formatBytes(o.live_sst_bytes_after)} ` +
        `(${formatBytes(Math.max(0, o.live_sst_bytes_before - o.live_sst_bytes_after))} reclaimed) in ${(o.duration_ms / 1000).toFixed(1)}s.`
    );
  }
}

interface RetentionResponse {
  branch: string;
  stored: RetentionPolicy | null;
  effective: RetentionPolicy;
}

function describe(p: RetentionPolicy | null | undefined): string {
  if (!p) return 'none';
  const parts: string[] = [];
  if (p.keep_days !== undefined && p.keep_days !== null) parts.push(`${p.keep_days} days`);
  if (p.keep_revisions !== undefined && p.keep_revisions !== null) parts.push(`${p.keep_revisions} revisions`);
  return parts.length ? `keep ${parts.join(' or ')}` : 'keep everything';
}

export async function repoRetention(
  repo: string,
  options: RetentionOptions = {},
  fetchImpl?: FetchLike
): Promise<void> {
  const query = options.branch ? `?branch=${encodeURIComponent(options.branch)}` : '';
  const path = `${base(options.tenant, repo)}/retention${query}`;
  const keepDays = parseCount('--keep-days', options.keepDays);
  const keepRevisions = parseCount('--keep-revisions', options.keepRevisions);

  let res;
  if (options.clear) {
    res = await apiCall<RetentionResponse>(path, { method: 'DELETE', fetchImpl });
  } else if (keepDays !== undefined || keepRevisions !== undefined) {
    const body: RetentionPolicy = {};
    if (keepDays !== undefined) body.keep_days = keepDays;
    if (keepRevisions !== undefined) body.keep_revisions = keepRevisions;
    res = await apiCall<RetentionResponse>(path, { method: 'PUT', body, fetchImpl });
  } else {
    res = await apiCall<RetentionResponse>(path, { fetchImpl });
  }
  if (!res.ok || !res.data) {
    throw new Error(`Retention of '${repo}' failed: ${res.errorMessage}`);
  }
  if (options.json) {
    console.log(JSON.stringify(res.data, null, 2));
    return;
  }
  const scope = res.data.branch === '*' ? 'all branches' : `branch ${res.data.branch}`;
  console.log(`${repo} (${scope}): stored policy ${describe(res.data.stored)}; effective ${describe(res.data.effective)}.`);
}
