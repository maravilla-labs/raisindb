import { afterEach, describe, expect, it, vi } from 'vitest';
import { formatBytes, repoGc, repoRetention } from './storage.js';

function fakeFetch(status: number, payload: unknown) {
  const calls: { url: string; init: RequestInit }[] = [];
  const impl = (async (url: string, init: RequestInit) => {
    calls.push({ url, init });
    return new Response(JSON.stringify(payload), { status });
  }) as unknown as typeof fetch;
  return { impl, calls };
}

const outcome = {
  dry_run: true,
  versions_deleted: 12,
  bytes_deleted: 4096,
  oplog_entries_deleted: 0,
  jobs_deleted: 0,
  orphaned_blobs: ['2026/09/29/aaaaaaaaaaaaaaaaaaaaa.rap'],
  blobs_deleted: 0,
  blob_bytes_deleted: 0,
  live_sst_bytes_before: 10,
  live_sst_bytes_after: 10,
  duration_ms: 5,
  column_families: { nodes: { versions_deleted: 12, bytes_deleted: 4096 } },
};

afterEach(() => vi.restoreAllMocks());

describe('repoGc', () => {
  it('posts the retention override to the repository GC endpoint', async () => {
    vi.spyOn(console, 'log').mockImplementation(() => {});
    const { impl, calls } = fakeFetch(200, outcome);
    await repoGc('website', { dryRun: true, keepDays: '3' }, impl);
    expect(calls[0].url).toMatch(/\/api\/admin\/management\/database\/default\/website\/history\/gc$/);
    expect(calls[0].init.method).toBe('POST');
    expect(JSON.parse(String(calls[0].init.body))).toEqual({ dry_run: true, keep_days: 3 });
  });

  it('rejects a malformed count before calling the server', async () => {
    const { impl, calls } = fakeFetch(200, outcome);
    await expect(repoGc('website', { keepRevisions: 'many' }, impl)).rejects.toThrow(/--keep-revisions/);
    expect(calls).toHaveLength(0);
  });

  it('surfaces a server error', async () => {
    const { impl } = fakeFetch(403, { error: 'forbidden' });
    await expect(repoGc('website', {}, impl)).rejects.toThrow(/forbidden/);
  });
});

describe('repoRetention', () => {
  it('reads, sets and clears the policy with GET, PUT and DELETE', async () => {
    vi.spyOn(console, 'log').mockImplementation(() => {});
    const resp = { branch: '*', stored: null, effective: { keep_days: 7, keep_revisions: 100 } };

    let f = fakeFetch(200, resp);
    await repoRetention('website', {}, f.impl);
    expect(f.calls[0].init.method).toBe('GET');

    f = fakeFetch(200, resp);
    await repoRetention('website', { branch: 'main', keepRevisions: '50' }, f.impl);
    expect(f.calls[0].url).toMatch(/history\/retention\?branch=main$/);
    expect(f.calls[0].init.method).toBe('PUT');
    expect(JSON.parse(String(f.calls[0].init.body))).toEqual({ keep_revisions: 50 });

    f = fakeFetch(200, resp);
    await repoRetention('website', { clear: true }, f.impl);
    expect(f.calls[0].init.method).toBe('DELETE');
  });
});

describe('formatBytes', () => {
  it('scales units', () => {
    expect(formatBytes(512)).toBe('512 B');
    expect(formatBytes(1536)).toBe('1.5 KB');
    expect(formatBytes(3 * 1024 ** 3)).toBe('3.0 GB');
  });
});
