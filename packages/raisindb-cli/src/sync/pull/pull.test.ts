import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import fs from 'fs';
import os from 'os';
import path from 'path';
import crypto from 'crypto';
import yaml from 'yaml';
import { FakeServer } from './fake-server.js';
import { runPull, type PullOptions } from './run.js';
import { Progress } from './report.js';
import type { SyncConfig } from '../config.js';
import type { JsonObject } from './normalize.js';

const config: SyncConfig = {
  version: 1,
  server: 'http://live.example',
  repository: 'site',
  branch: 'main',
  remote_path: '/',
  conflict_strategy: 'prompt',
  ignore: [],
};

const sha = (b: Buffer | string) => crypto.createHash('sha256').update(b).digest('hex');

describe('raisindb pull', () => {
  let dir: string;
  let server: FakeServer;
  let lines: string[];

  beforeEach(() => {
    dir = fs.mkdtempSync(path.join(os.tmpdir(), 'raisindb-pull-'));
    fs.writeFileSync(path.join(dir, 'manifest.yaml'), 'name: test\nversion: 1.0.0\n');
    server = new FakeServer();
    lines = [];
  });
  afterEach(() => fs.rmSync(dir, { recursive: true, force: true }));

  const write = (rel: string, text: string) => {
    const full = path.join(dir, rel);
    fs.mkdirSync(path.dirname(full), { recursive: true });
    fs.writeFileSync(full, text);
  };
  const read = (rel: string) => fs.readFileSync(path.join(dir, rel), 'utf-8');
  const exists = (rel: string) => fs.existsSync(path.join(dir, rel));
  const pull = (extra: Partial<PullOptions> = {}) =>
    runPull({
      packageDir: dir,
      config,
      client: server,
      log: (s) => lines.push(s),
      progress: new Progress(false, { write: () => true } as unknown as NodeJS.WriteStream),
      ...extra,
    });

  it('pulls a deep tree (5 levels) across many pages, and counts match the server', async () => {
    write('content/stories/site/.node.yaml', 'node_type: studio:Story\nproperties:\n  title: Site\n');
    server.add({ workspace: 'stories', path: '/site', node_type: 'studio:Story', properties: { title: 'Site' } });
    let n = 1;
    for (let a = 0; a < 6; a++) {
      for (let b = 0; b < 6; b++) {
        for (let c = 0; c < 6; c++) {
          const p = `/site/a${a}/b${b}/c${c}`;
          for (const q of [`/site/a${a}`, `/site/a${a}/b${b}`, p]) {
            if (!server.nodes.some((x) => x.path === q)) {
              server.add({ workspace: 'stories', path: q, node_type: 'studio:Page', properties: { title: q } });
              n++;
            }
          }
          server.add({ workspace: 'stories', path: `${p}/leaf`, node_type: 'studio:Page', properties: { title: 'deep' } });
          n++;
        }
      }
    }
    expect(n).toBeGreaterThan(400);

    const report = await pull({ pageSize: 37 });

    expect(report.aborted).toBeUndefined();
    expect(report.serverNodes).toBe(n);
    expect(report.counts.create).toBe(n - 1);
    expect(report.counts.unchanged).toBe(1);
    const leaf = yaml.parse(read('content/stories/site/a5/b5/c5/leaf/.node.yaml'));
    expect(leaf).toEqual({ node_type: 'studio:Page', properties: { title: 'deep' } });
    // Every page was a keyset page: ceil(n / 37) scans plus one COUNT.
    expect(server.sqlCalls).toBeGreaterThanOrEqual(Math.ceil(n / 37));
  });

  it('is a no-op the second time', async () => {
    write('content/stories/site/.node.yaml', 'node_type: studio:Story\nproperties:\n  title: Old\n');
    server.add({ workspace: 'stories', path: '/site', node_type: 'studio:Story', properties: { title: 'Site' } });
    server.add({ workspace: 'stories', path: '/site/home', node_type: 'studio:Page', properties: { title: 'Home' } });

    await pull({ force: true });
    lines = [];
    const second = await pull();
    expect(second.filesWritten).toBe(0);
    expect(second.counts.unchanged).toBe(2);
    expect(second.counts.create ?? 0).toBe(0);
  });

  it('streams binaries to disk, checks content_hash, and does not re-download unchanged ones', async () => {
    const bytes = crypto.randomBytes(300_000);
    write('content/assets/web/.node.yaml', 'node_type: raisin:Folder\nproperties:\n  title: Web\n');
    server.add({ workspace: 'assets', path: '/web', node_type: 'raisin:Folder', properties: { title: 'Web' } });
    server.add({
      workspace: 'assets',
      path: '/web/hero.jpg',
      node_type: 'raisin:Asset',
      bytes,
      properties: {
        title: 'Hero on ice',
        alt_text: 'Players on the ice',
        content_hash: sha(bytes),
        file_size: bytes.length,
        file: { name: 'hero.jpg', metadata: { storage_key: 'k1' } },
        renditions: { thumb: { status: 'failed' } },
      },
    });

    await pull();
    expect(sha(fs.readFileSync(path.join(dir, 'content/assets/web/hero.jpg')))).toBe(sha(bytes));
    expect(fs.readdirSync(path.join(dir, 'content/assets/web')).some((f) => f.endsWith('.raisindb-partial'))).toBe(false);
    const meta = yaml.parse(read('content/assets/web/.node.hero.jpg.yaml'));
    expect(meta).toEqual({ node_type: 'raisin:Asset', title: 'Hero on ice', properties: { alt_text: 'Players on the ice' } });

    server.downloads = [];
    const again = await pull();
    expect(server.downloads).toEqual([]);
    expect(again.filesWritten).toBe(0);
  });

  it('refuses a download whose bytes do not match content_hash', async () => {
    write('content/assets/web/.node.yaml', 'node_type: raisin:Folder\nproperties:\n  title: Web\n');
    server.add({ workspace: 'assets', path: '/web', node_type: 'raisin:Folder', properties: { title: 'Web' } });
    server.add({
      workspace: 'assets', path: '/web/a.bin', node_type: 'raisin:Asset', bytes: Buffer.from('actual'),
      properties: { content_hash: sha('expected'), file: { metadata: { storage_key: 'k' } } },
    });
    const report = await pull();
    expect(report.errors.join()).toMatch(/content_hash/);
    expect(exists('content/assets/web/a.bin')).toBe(false);
  });

  it('writes translation overlays with only what the locale changes', async () => {
    server.locales = { defaultLanguage: 'de', supported: ['de', 'en', 'fr'] };
    write('content/stories/site/.node.yaml', 'node_type: studio:Story\nproperties:\n  title: Seite\n');
    const base: JsonObject = {
      title: 'Seite',
      slug: 'seite',
      content: [
        { uuid: 'b1', element_type: 'x:Text', text: 'Hallo', style: 'big' },
        { uuid: 'b2', element_type: 'x:Text', text: 'Gleich' },
      ],
    };
    server.add({
      workspace: 'stories', path: '/site', node_type: 'studio:Story', properties: base,
      locales: {
        en: {
          ...base,
          title: 'Page',
          content: [
            { uuid: 'b1', element_type: 'x:Text', text: 'Hello', style: 'big' },
            { uuid: 'b2', element_type: 'x:Text', text: 'Gleich' },
          ],
        },
        fr: null,
      },
    });
    // A locale view with no rows at all reads as "not available", never as
    // "everything hidden" — so the fr view needs one visible node.
    server.add({ workspace: 'stories', path: '/site/home', node_type: 'studio:Page', properties: { title: 'Start' } });

    await pull({ force: true });
    expect(yaml.parse(read('content/stories/site/.node.en.yaml'))).toEqual({
      title: 'Page',
      content: [{ uuid: 'b1', text: 'Hello' }],
    });
    expect(yaml.parse(read('content/stories/site/.node.fr.yaml'))).toEqual({ hidden: true });
    expect(exists('content/stories/site/.node.de.yaml')).toBe(false);

    const again = await pull();
    expect(again.filesWritten).toBe(0);
  });

  it('decides three-way: server change → update, local change → kept, both → conflict', async () => {
    const file = 'content/stories/site/.node.yaml';
    write(file, '# Why the title is what it is.\nnode_type: studio:Story\nproperties:\n  title: Site\n  domain: "{env:SITE_DOMAIN:-localhost}"\n');
    const node = server.add({
      workspace: 'stories', path: '/site', node_type: 'studio:Story',
      properties: { title: 'Site', domain: 'rhinos.example' },
    }).nodes[0];

    // First pull: equal modulo the env token → unchanged, baseline recorded.
    let r = await pull();
    expect(r.counts.unchanged).toBe(1);

    // Server changes, local does not → updated in place; comment and token survive.
    node.properties = { title: 'Live title', domain: 'rhinos.example' };
    r = await pull();
    expect(r.counts.update).toBe(1);
    expect(read(file)).toContain('# Why the title is what it is.');
    expect(read(file)).toContain('{env:SITE_DOMAIN:-localhost}');
    expect(yaml.parse(read(file)).properties.title).toBe('Live title');

    // Local changes, server does not → kept.
    write(file, read(file).replace('Live title', 'My draft'));
    r = await pull();
    expect(r.counts['local-edit']).toBe(1);
    expect(read(file)).toContain('My draft');

    // Both change → conflict, left alone…
    node.properties = { title: 'Editor changed it again', domain: 'rhinos.example' };
    r = await pull({ diff: true });
    expect(r.counts.conflict).toBe(1);
    expect(r.conflictsUnresolved).toBe(1);
    expect(read(file)).toContain('My draft');
    expect(lines.join('\n')).toMatch(/-  title: My draft\n\+  title: Editor changed it again/);

    // …until --force takes the server's.
    r = await pull({ force: true });
    expect(r.counts['conflict-server']).toBe(1);
    expect(yaml.parse(read(file)).properties.title).toBe('Editor changed it again');
  });

  it('asks about a conflict when interactive', async () => {
    write('content/stories/site/.node.yaml', 'node_type: studio:Story\nproperties:\n  title: Mine\n');
    server.add({ workspace: 'stories', path: '/site', node_type: 'studio:Story', properties: { title: 'Theirs' } });
    const asked: string[] = [];
    await pull({
      askConflict: async (p) => {
        asked.push(p.file.rel);
        return 'server';
      },
    });
    expect(asked).toEqual(['stories/site/.node.yaml']);
    expect(yaml.parse(read('content/stories/site/.node.yaml')).properties.title).toBe('Theirs');
  });

  it('--dry-run reports the plan and writes nothing', async () => {
    write('content/stories/site/.node.yaml', 'node_type: studio:Story\nproperties:\n  title: Site\n');
    server.add({ workspace: 'stories', path: '/site', node_type: 'studio:Story', properties: { title: 'Site' } });
    server.add({ workspace: 'stories', path: '/site/new', node_type: 'studio:Page', properties: { title: 'New' } });
    const before = fs.readdirSync(path.join(dir, 'content/stories/site'));

    const r = await pull({ dryRun: true });
    expect(r.counts.create).toBe(1);
    expect(lines.join('\n')).toContain('+ stories/site/new/.node.yaml');
    expect(lines.join('\n')).toContain('[dry run]');
    expect(fs.readdirSync(path.join(dir, 'content/stories/site'))).toEqual(before);
    expect(exists('.raisindb/pull-state.json')).toBe(false);
  });

  it('stops on an empty read instead of treating it as "everything was deleted"', async () => {
    write('content/stories/site/.node.yaml', 'node_type: studio:Story\nproperties:\n  title: Site\n');
    write('content/stories/site/home/.node.yaml', 'node_type: studio:Page\nproperties:\n  title: Home\n');
    server.add({ workspace: 'stories', path: '/site', node_type: 'studio:Story', properties: { title: 'Site' } });
    server.anonymous = true;

    const r = await pull({ delete: true });
    expect(r.aborted).toMatch(/EMPTY result, not an error/);
    expect(exists('content/stories/site/home/.node.yaml')).toBe(true);
  });

  it('reports nodes gone from the server, and deletes them only with --delete', async () => {
    write('content/stories/site/.node.yaml', 'node_type: studio:Story\nproperties:\n  title: Site\n');
    write('content/stories/site/old/.node.yaml', 'node_type: studio:Page\nproperties:\n  title: Old\n');
    server.add({ workspace: 'stories', path: '/site', node_type: 'studio:Story', properties: { title: 'Site' } });

    let r = await pull();
    expect(r.counts.orphan).toBe(1);
    expect(exists('content/stories/site/old/.node.yaml')).toBe(true);
    r = await pull({ delete: true });
    expect(exists('content/stories/site/old/.node.yaml')).toBe(false);
  });

  it('writes references in path form and drops server bookkeeping', async () => {
    write('content/stories/site/.node.yaml', 'node_type: studio:Story\nproperties: {}\n');
    server.add({
      workspace: 'stories', path: '/site', node_type: 'studio:Story', archetype: 'studio:StorySettings',
      properties: {
        $supertypes: ['studio:Story'],
        $mixins: [],
        __extract_status: 'ok',
        brand: { 'raisin:ref': 'Xy12uuid', 'raisin:workspace': 'branding', 'raisin:path': '/rhinos' },
      },
    });
    await pull({ force: true });
    expect(yaml.parse(read('content/stories/site/.node.yaml'))).toEqual({
      node_type: 'studio:Story',
      archetype: 'studio:StorySettings',
      properties: { brand: { 'raisin:ref': '/rhinos', 'raisin:workspace': 'branding' } },
    });
    // A package ref in path form equals the server's uuid form.
    expect((await pull()).counts.unchanged).toBe(1);
  });

  it('respects .raisin-sync.yaml: replace is package-owned, push_only never pulled', async () => {
    write('.raisin-sync.yaml', [
      'defaults: {mode: skip}',
      'filters:',
      '  - root: /functions',
      '    mode: replace',
      '  - root: /stories/site/private',
      '    direction: push_only',
    ].join('\n'));
    write('content/functions/lib/fn/.node.yaml', 'node_type: raisin:Function\nproperties:\n  title: Mine\n');
    write('content/stories/site/.node.yaml', 'node_type: studio:Story\nproperties:\n  title: Site\n');
    server.add({ workspace: 'functions', path: '/lib/fn', node_type: 'raisin:Function', properties: { title: 'Deployed' } });
    server.add({ workspace: 'stories', path: '/site', node_type: 'studio:Story', properties: { title: 'Site' } });
    server.add({ workspace: 'stories', path: '/site/private', node_type: 'studio:Page', properties: { title: 'P' } });

    let r = await pull({ force: true });
    expect(r.counts.owned).toBe(1);
    expect(r.counts.policy).toBe(1);
    expect(read('content/functions/lib/fn/.node.yaml')).toContain('Mine');
    expect(exists('content/stories/site/private/.node.yaml')).toBe(false);

    r = await pull({ force: true, includeOwned: true });
    expect(read('content/functions/lib/fn/.node.yaml')).toContain('Deployed');
  });

  it('pulls only what the package contains, and names what it left out', async () => {
    write('content/stories/site/.node.yaml', 'node_type: studio:Story\nproperties:\n  title: Site\n');
    server.add({ workspace: 'stories', path: '/site', node_type: 'studio:Story', properties: { title: 'Site' } });
    server.add({ workspace: 'stories', path: '/blueprints', node_type: 'studio:Folder', properties: { title: 'B' } });

    await pull();
    expect(exists('content/stories/blueprints/.node.yaml')).toBe(false);
    expect(lines.join('\n')).toContain('Not part of the package, so not pulled — stories: stories:/blueprints');

    await pull({ paths: ['stories:/'] });
    expect(exists('content/stories/blueprints/.node.yaml')).toBe(true);
  });
});

describe('raisindb pull: repository and remote_path checks', () => {
  let dir: string;
  let server: FakeServer;
  let lines: string[];
  beforeEach(() => {
    dir = fs.mkdtempSync(path.join(os.tmpdir(), 'raisindb-pull-repo-'));
    fs.writeFileSync(path.join(dir, 'manifest.yaml'), 'name: t\nversion: 1.0.0\n');
    fs.mkdirSync(path.join(dir, 'content/stories/site'), { recursive: true });
    fs.writeFileSync(path.join(dir, 'content/stories/site/.node.yaml'), 'node_type: studio:Story\nproperties: {}\n');
    server = new FakeServer();
    server.add({ workspace: 'stories', path: '/site', node_type: 'studio:Story', properties: {} });
    lines = [];
  });
  afterEach(() => fs.rmSync(dir, { recursive: true, force: true }));
  const pull = (cfg: Partial<SyncConfig> = {}, extra: Partial<PullOptions> = {}) =>
    runPull({
      packageDir: dir,
      config: { ...config, ...cfg },
      client: server,
      serverLabel: 'http://live.example',
      log: (s) => lines.push(s),
      progress: new Progress(false, { write: () => true } as unknown as NodeJS.WriteStream),
      ...extra,
    });

  it('names the repositories that exist when the configured one does not', async () => {
    server.repos = ['studio'];
    const r = await pull({ repository: 'default' });
    expect(r.aborted).toBe(
      "Repository 'default' not found on http://live.example. Available: studio. Pass --repo <name>, or fix `repository:` in .raisindb-cli.yaml."
    );
    expect(server.sqlCalls).toBe(0);
  });

  it('tells refused credentials and an anonymous empty listing apart', async () => {
    server.repoListingStatus = 401;
    expect((await pull()).aborted).toMatch(/refused these credentials \(HTTP 401\)/);

    server.repoListingStatus = 200;
    server.anonymous = true;
    expect((await pull({}, { authenticated: false })).aborted).toMatch(/No credentials were sent/);

    // With a token, an empty listing is ambiguous: warn, then let the
    // empty-read guard decide.
    const r = await pull({}, { authenticated: true });
    expect(lines.join('\n')).toMatch(/lists no repositories for these credentials/);
    expect(r.aborted).toMatch(/EMPTY result/);
  });

  it('warns when remote_path matches nothing, and is quiet when it does', async () => {
    await pull({ remote_path: '/baden-rhinos' });
    expect(lines.join('\n')).toMatch(/remote_path '\/baden-rhinos' in .raisindb-cli.yaml matches no node in stories/);
    lines = [];
    await pull({ remote_path: '/site' });
    expect(lines.join('\n')).not.toMatch(/remote_path/);
  });
});

describe('raisindb pull: server-controlled names never leave the package', () => {
  let root: string;
  let dir: string;
  let server: FakeServer;
  let lines: string[];
  beforeEach(() => {
    root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'raisindb-pull-sec-')));
    dir = path.join(root, 'pkg');
    fs.mkdirSync(path.join(dir, 'content/stories/site'), { recursive: true });
    fs.writeFileSync(path.join(dir, 'manifest.yaml'), 'name: t\nversion: 1.0.0\n');
    fs.writeFileSync(path.join(dir, 'content/stories/site/.node.yaml'), 'node_type: studio:Story\nproperties: {}\n');
    server = new FakeServer();
    server.add({ workspace: 'stories', path: '/site', node_type: 'studio:Story', properties: {} });
    lines = [];
  });
  afterEach(() => fs.rmSync(root, { recursive: true, force: true }));
  const pull = (extra: Partial<PullOptions> = {}) =>
    runPull({
      packageDir: dir,
      config,
      client: server,
      log: (s) => lines.push(s),
      progress: new Progress(false, { write: () => true } as unknown as NodeJS.WriteStream),
      ...extra,
    });
  /** Every file under `root` outside the package dir. */
  const outside = () => {
    const out: string[] = [];
    const walk = (d: string) => {
      for (const e of fs.readdirSync(d, { withFileTypes: true })) {
        const p = path.join(d, e.name);
        if (p === dir) continue;
        if (e.isDirectory()) walk(p);
        else out.push(path.relative(root, p));
      }
    };
    walk(root);
    return out;
  };

  const evil = [
    '/site/../../../escaped',
    '/site/..',
    '/site/a/../../../../escaped',
    '/site/..\\..\\escaped',
    '/site/%2e%2e',
    '/site/%252e%252e',
    '/site/C:evil',
    '/site/nul\u0000byte',
    '/site/line\nbreak',
    '/site//double',
    '/site/trailing.',
  ];

  for (const p of evil) {
    it(`refuses node path ${JSON.stringify(p)}`, async () => {
      server.add({ workspace: 'stories', path: p, node_type: 'studio:Page', properties: { title: 'x' } });
      const r = await pull({ force: true, paths: ['stories:/'] });
      expect(r.counts.refused).toBeGreaterThanOrEqual(1);
      expect(outside()).toEqual([]);
      expect(lines.join('\n')).toContain('REFUSED');
    });
  }

  it('refuses a binary named with traversal, and an unsafe locale and workspace', async () => {
    server.locales = { defaultLanguage: 'de', supported: ['de', '../../x'] };
    server.add({
      workspace: 'stories', path: '/site/../../evil.bin', node_type: 'raisin:Asset', bytes: Buffer.from('x'),
      properties: { file: { metadata: { storage_key: 'k' } } },
    });
    server.add({
      workspace: 'stories', path: '/site/page', node_type: 'studio:Page', properties: { title: 'a' },
      locales: { '../../x': { title: 'b' } },
    });
    server.add({ workspace: '../evil', path: '/x', node_type: 'studio:Page', properties: {} });
    const r = await pull({ force: true, paths: ['stories:/', '../evil:/'] });
    expect(r.counts.refused).toBeGreaterThanOrEqual(3);
    expect(server.downloads).toEqual([]);
    expect(outside()).toEqual([]);
  });

  it('refuses a node whose name disagrees with its path', async () => {
    server.nodes.push({ workspace: 'stories', path: '/site/ok', node_type: 'studio:Page', properties: {} });
    const orig = server.sql.bind(server);
    server.sql = async (q, p) =>
      (await orig(q, p)).map((r) => (r.path === '/site/ok' ? { ...r, name: '../../escaped' } : r));
    const r = await pull({ force: true });
    expect(r.counts.refused).toBe(1);
    expect(outside()).toEqual([]);
  });

  it('does not write through a symlinked directory planted in the package', async () => {
    const target = path.join(root, 'elsewhere');
    fs.mkdirSync(target);
    fs.symlinkSync(target, path.join(dir, 'content/stories/site/linked'));
    server.add({ workspace: 'stories', path: '/site/linked', node_type: 'studio:Page', properties: { title: 'x' } });
    server.add({ workspace: 'stories', path: '/site/linked/child', node_type: 'studio:Page', properties: { title: 'y' } });
    const r = await pull({ force: true });
    expect(r.counts.refused).toBe(2);
    expect(fs.readdirSync(target)).toEqual([]);
  });

  it('does not write through a symlinked file, or a planted temp-file symlink', async () => {
    const victim = path.join(root, 'victim.txt');
    fs.writeFileSync(victim, 'original');
    fs.mkdirSync(path.join(dir, 'content/stories/site/page'));
    fs.symlinkSync(victim, path.join(dir, 'content/stories/site/page/.node.yaml'));
    fs.mkdirSync(path.join(dir, 'content/stories/site/other'));
    fs.symlinkSync(victim, path.join(dir, 'content/stories/site/other/.node.yaml.raisindb-partial'));
    server.add({ workspace: 'stories', path: '/site/page', node_type: 'studio:Page', properties: { title: 'x' } });
    server.add({ workspace: 'stories', path: '/site/other', node_type: 'studio:Page', properties: { title: 'y' } });
    const r = await pull({ force: true });
    expect(r.counts.refused).toBe(1);
    expect(fs.readFileSync(victim, 'utf-8')).toBe('original');
    expect(yaml.parse(fs.readFileSync(path.join(dir, 'content/stories/site/other/.node.yaml'), 'utf-8')).properties.title).toBe('y');
  });

  it('--delete never deletes through a symlink', async () => {
    const target = path.join(root, 'keep');
    fs.mkdirSync(path.join(target, 'gone'), { recursive: true });
    fs.writeFileSync(path.join(target, 'gone/.node.yaml'), 'node_type: x\n');
    fs.symlinkSync(target, path.join(dir, 'content/stories/site/linked'));
    await pull({ delete: true });
    expect(fs.existsSync(path.join(target, 'gone/.node.yaml'))).toBe(true);
  });
});
