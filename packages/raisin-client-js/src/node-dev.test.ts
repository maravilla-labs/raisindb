import { describe, expect, it } from 'vitest';
import { NodeDevApi, type NodeDevTransport } from './node-dev';

function transport(calls: Array<{ method: string; path: string; body?: unknown }>): NodeDevTransport {
  return {
    request: async <T>(method: string, path: string, body?: unknown) => {
      calls.push({ method, path, body });
      return { ok: true } as T;
    },
  };
}

describe('NodeDevApi', () => {
  it('posts one method per route with the branch in the query', async () => {
    const calls: Array<{ method: string; path: string; body?: unknown }> = [];
    const api = new NodeDevApi('studio', transport(calls));
    await api.read('views/board', { roots: [{ workspace: 'apps', path: '/libs/x' }], branch: 'draft/1' });
    expect(calls[0]).toMatchObject({
      method: 'POST',
      path: '/api/node-dev/studio/read?branch=draft%2F1',
      body: { target: 'views/board', roots: [{ workspace: 'apps', path: '/libs/x' }] },
    });
  });

  it('maps changeset options onto the wire contract', async () => {
    const calls: Array<{ method: string; path: string; body?: unknown }> = [];
    const api = new NodeDevApi('studio', transport(calls));
    await api.apply({
      workspace: 'apps',
      idempotencyKey: 'k1',
      ops: [{ op: 'move', target: '/a', to_parent: '/b', expected_revision: 'r1' }],
    });
    await api.commit('c'.repeat(32), 'sha256:x');
    await api.mergeBranch('main', { dryRun: true });
    expect(calls[0].path).toBe('/api/node-dev/studio/apply');
    expect(calls[0].body).toMatchObject({ idempotency_key: 'k1', workspace: 'apps' });
    expect(calls[1].body).toMatchObject({ changeset_id: 'c'.repeat(32), expected_digest: 'sha256:x' });
    expect(calls[2].body).toMatchObject({ target: 'main', dry_run: true });
  });
});
