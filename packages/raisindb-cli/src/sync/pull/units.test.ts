import { describe, it, expect } from 'vitest';
import fs from 'fs';
import os from 'os';
import path from 'path';
import { computeOverlay, equalModuloEnv, overlaysEqual } from './normalize.js';
import { buildLocalIndex, defaultRoots, parseRootSpec, policyFor } from './scope.js';
import { mergeNodeYaml } from './yaml-merge.js';
import { createHttpClient, scanSubtree } from './remote.js';

describe('pull scope', () => {
  it('parses root specs, including namespaced workspaces', () => {
    expect(parseRootSpec('stories:/site/')).toEqual({ workspace: 'stories', path: '/site' });
    expect(parseRootSpec('raisin:access_control:/roles')).toEqual({ workspace: 'raisin:access_control', path: '/roles' });
    expect(parseRootSpec('sponsoren')).toEqual({ workspace: 'sponsoren', path: '/' });
  });

  it('anchors default roots at what the package defines', () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'raisindb-scope-'));
    const w = (rel: string) => {
      fs.mkdirSync(path.dirname(path.join(dir, rel)), { recursive: true });
      fs.writeFileSync(path.join(dir, rel), 'node_type: x\n');
    };
    w('stories/site/.node.yaml');
    w('stories/site/home/.node.yaml');
    w('apps/local/appdefs/kader/.node.yaml'); // `local`, `appdefs`: bare directories
    w('assets/website/teams/a.jpg'); // bare binary → its top-level folder
    w('functions/lib/fn/.node.yaml');
    w('functions/lib/fn/index.js'); // below a defined node → covered by it
    w('_raisin__access_control/roles/editor/.node.yaml');
    const index = buildLocalIndex(dir, () => false, false);
    expect(defaultRoots(index).map((r) => `${r.workspace}:${r.path}`)).toEqual([
      'apps:/local/appdefs/kader',
      'assets:/website',
      'functions:/lib/fn',
      'raisin:access_control:/roles/editor',
      'stories:/site',
    ]);
    fs.rmSync(dir, { recursive: true, force: true });
  });

  it('reads the install policy the way the server does (last match wins)', () => {
    const policy = {
      defaults: { mode: 'skip' },
      filters: [
        { root: '/stories', mode: 'replace' },
        { root: '/stories/site/news', mode: 'skip' },
        { root: '/stories/site/drafts', direction: 'local_only' },
        { root: '/people', exclude: ['**/private/**'] },
      ],
    };
    expect(policyFor(policy, 'stories', '/site/home')).toMatchObject({ pull: true, owned: true });
    expect(policyFor(policy, 'stories', '/site/news/a')).toMatchObject({ pull: true, owned: false });
    expect(policyFor(policy, 'stories', '/site/drafts/x')).toMatchObject({ pull: false });
    expect(policyFor(policy, 'people', '/a/private/b')).toMatchObject({ pull: false });
    expect(policyFor(policy, 'tags', '/x')).toMatchObject({ pull: true, owned: false });
    expect(policyFor({ defaults: {}, filters: [] }, 'tags', '/x')).toMatchObject({ owned: true });
    expect(policyFor(null, 'tags', '/x')).toMatchObject({ pull: true, owned: false });
  });
});

describe('pull normalisation', () => {
  it('treats an env token as matching any server value', () => {
    expect(equalModuloEnv({ url: '{env:SITE:-x}', a: 1 }, { url: 'https://live', a: 1 })).toBe(true);
    expect(equalModuloEnv({ url: '{env:SITE:-x}', a: 1 }, { url: 'https://live', a: 2 })).toBe(false);
  });

  it('reconstructs an overlay from base and localized properties', () => {
    const base = { title: 'Hallo', slug: 's', blocks: [{ uuid: 'u', element_type: 't', text: 'a', n: [{ uuid: 'v', t: 'x' }] }] };
    const loc = { title: 'Hello', slug: 's-en', blocks: [{ uuid: 'u', element_type: 't', text: 'a', n: [{ uuid: 'v', t: 'y' }] }] };
    const overlay = computeOverlay(base, loc);
    expect(overlay).toEqual({ title: 'Hello', blocks: [{ uuid: 'u', n: [{ uuid: 'v', t: 'y' }] }] });
    expect(overlaysEqual(overlay, { blocks: [{ uuid: 'u', n: [{ uuid: 'v', t: 'y' }] }], title: 'Hello' })).toBe(true);
  });

  it('merges into existing YAML keeping comments, tokens and unchanged formatting', () => {
    const text = [
      '# Top comment',
      'node_type: studio:Page',
      'properties:',
      '  title: Old # why',
      '  url: "{env:URL:-http://localhost}"',
      '  blocks:',
      '    - uuid: a',
      '      text: keep # block comment',
      '    - uuid: b',
      '      text: change',
      '',
    ].join('\n');
    const out = mergeNodeYaml(text, {
      node_type: 'studio:Page',
      properties: {
        title: 'New',
        url: 'https://live.example',
        blocks: [{ uuid: 'b', text: 'changed' }, { uuid: 'a', text: 'keep' }],
        added: 1,
      },
    });
    expect(out).toContain('# Top comment');
    expect(out).toContain('"{env:URL:-http://localhost}"');
    expect(out).toContain('text: keep # block comment');
    expect(out).toContain('title: New');
    expect(out).toMatch(/uuid: b\n\s+text: changed\n\s+- uuid: a/);
    expect(out).toContain('added: 1');
  });
});

describe('pull HTTP client', () => {
  it('pages with bound parameters and streams downloads', async () => {
    const calls: Array<{ url: string; body?: unknown }> = [];
    const fetchImpl = (async (url: string, init?: RequestInit) => {
      const body = init?.body ? JSON.parse(String(init.body)) : undefined;
      calls.push({ url, body });
      if (url.endsWith('/raisin:cmd/download')) {
        return new Response(new Blob([Buffer.alloc(70_000, 7)]).stream(), { status: 200 });
      }
      const after = body.params?.[1];
      const rows = after ? [] : [{ id: 'a', path: '/x', name: 'x', node_type: 't', properties: '{"k":1}' }];
      return new Response(JSON.stringify({ rows }), { status: 200 });
    }) as unknown as typeof fetch;

    const client = createHttpClient({ server: 'https://live/', repo: 'r', branch: 'main', token: 't', fetchImpl });
    const nodes = await scanSubtree(client, 'stories', '/site', { pageSize: 1 });
    expect(nodes).toHaveLength(1);
    expect(nodes[0].properties).toEqual({ k: 1 });
    expect(calls[0].url).toBe('https://live/api/sql/r/main');
    expect(calls[0].body).toMatchObject({ params: ['/site'] });
    expect(calls[1].body).toMatchObject({ params: ['/site', 'a'] });

    const dest = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'raisindb-dl-')), 'f.bin');
    const got = await client.download('assets', '/a b/f.bin', dest);
    expect(got.size).toBe(70_000);
    expect(calls[2].url).toBe('https://live/api/repository/r/main/head/assets/a%20b/f.bin/raisin:cmd/download');
  });
});

describe('pull path safety', () => {
  it('rejects unsafe segments and accepts ordinary names', async () => {
    const { segmentProblem, isSafeLocale, isSafeWorkspace, resolveInside } = await import('./safe-path.js');
    for (const bad of ['', '.', '..', 'a/b', 'a\\b', '%2e%2e', '%252E%252E', 'C:x', 'a\u0000b', 'x.', 'x ']) {
      expect(segmentProblem(bad)).not.toBeNull();
    }
    for (const ok of ['home', 'index.js', '.node.yaml', 'Hobbymannschaft-Header.webp', 'ä ü.jpg']) {
      expect(segmentProblem(ok)).toBeNull();
    }
    expect(isSafeLocale('de')).toBe(true);
    expect(isSafeLocale('pt-BR')).toBe(true);
    expect(isSafeLocale('../x')).toBe(false);
    expect(isSafeWorkspace('raisin:access_control')).toBe(true);
    expect(isSafeWorkspace('../x')).toBe(false);

    const base = fs.mkdtempSync(path.join(os.tmpdir(), 'raisindb-safe-'));
    expect(() => resolveInside(base, '/etc/passwd')).toThrow(/absolute/);
    expect(() => resolveInside(base, 'C:/x')).toThrow(/absolute/);
    expect(() => resolveInside(base, 'a/../../x')).toThrow(/dot segment/);
    expect(() => resolveInside(base, 'a\\..\\..\\x')).toThrow(/separator/);
    expect(resolveInside(base, 'stories/a/.node.yaml')).toBe(path.join(fs.realpathSync(base), 'stories/a/.node.yaml'));
    fs.rmSync(base, { recursive: true, force: true });
  });
});
