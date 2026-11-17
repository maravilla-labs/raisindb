import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import fs from 'fs';
import path from 'path';
import os from 'os';
import AdmZip from 'adm-zip';
import {
  collectFiles,
  createIgnoreFilter,
  createZipPackage,
  DEFAULT_IGNORE_PATTERNS,
  formatInstallFailure,
  formatPostInstallResults,
  installVerdict,
  currentResults,
  retryNotice,
  INSTALL_POLL_TIMEOUT_MS,
} from './package.js';
import { countPostInstallHooks, type PackageSummary } from '../api.js';

function makeTempDir(): string {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'raisindb-cli-test-'));
}

function writeFile(base: string, relPath: string, content = '') {
  const full = path.join(base, relPath);
  fs.mkdirSync(path.dirname(full), { recursive: true });
  fs.writeFileSync(full, content);
}

describe('collectFiles', () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = makeTempDir();
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  });

  it('returns files relative to the base directory', () => {
    writeFile(tmpDir, 'manifest.yaml', 'name: test');
    writeFile(tmpDir, 'nodetypes/Post.yaml', '$type: NodeType');
    writeFile(tmpDir, 'workspaces/default/content.yaml', '');

    const files = collectFiles(tmpDir, tmpDir);

    expect(files).toContain('manifest.yaml');
    expect(files).toContain(path.join('nodetypes', 'Post.yaml'));
    expect(files).toContain(path.join('workspaces', 'default', 'content.yaml'));
  });

  it('returns empty array for empty directory', () => {
    const files = collectFiles(tmpDir, tmpDir);
    expect(files).toEqual([]);
  });
});

describe('createIgnoreFilter', () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = makeTempDir();
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  });

  it('ignores default patterns like .DS_Store and node_modules', () => {
    const ig = createIgnoreFilter(tmpDir);

    expect(ig.ignores('.DS_Store')).toBe(true);
    expect(ig.ignores('node_modules/foo/bar.js')).toBe(true);
    expect(ig.ignores('.git/config')).toBe(true);
  });

  it('respects .gitignore patterns', () => {
    writeFile(tmpDir, '.gitignore', 'dist/\n*.log');

    const ig = createIgnoreFilter(tmpDir);

    expect(ig.ignores('dist/index.js')).toBe(true);
    expect(ig.ignores('error.log')).toBe(true);
    expect(ig.ignores('src/index.ts')).toBe(false);
  });

  it('respects .rapignore patterns', () => {
    writeFile(tmpDir, '.rapignore', 'drafts/');

    const ig = createIgnoreFilter(tmpDir);

    expect(ig.ignores('drafts/readme.md')).toBe(true);
    expect(ig.ignores('content/page.yaml')).toBe(false);
  });

  it('always ignores .gitignore and .rapignore files themselves', () => {
    const ig = createIgnoreFilter(tmpDir);

    expect(ig.ignores('.gitignore')).toBe(true);
    expect(ig.ignores('.rapignore')).toBe(true);
  });

  it('allows normal files through', () => {
    const ig = createIgnoreFilter(tmpDir);

    expect(ig.ignores('manifest.yaml')).toBe(false);
    expect(ig.ignores('nodetypes/Post.yaml')).toBe(false);
  });
});

describe('createZipPackage', () => {
  let tmpDir: string;
  let outputDir: string;

  beforeEach(() => {
    tmpDir = makeTempDir();
    outputDir = makeTempDir();
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
    fs.rmSync(outputDir, { recursive: true, force: true });
  });

  it('creates a valid ZIP file with correct entries', () => {
    writeFile(tmpDir, 'manifest.yaml', 'name: test\nversion: 1.0.0');
    writeFile(tmpDir, 'nodetypes/Post.yaml', '$type: NodeType');

    const outputPath = path.join(outputDir, 'test.zip');
    createZipPackage(tmpDir, outputPath);

    expect(fs.existsSync(outputPath)).toBe(true);

    const zip = new AdmZip(outputPath);
    const entries = zip.getEntries().map((e) => e.entryName);

    expect(entries).toContain('manifest.yaml');
    expect(entries).toContain(path.join('nodetypes', 'Post.yaml'));
  });

  it('preserves file contents in the archive', () => {
    const content = 'name: my-package\nversion: 2.0.0';
    writeFile(tmpDir, 'manifest.yaml', content);

    const outputPath = path.join(outputDir, 'test.zip');
    createZipPackage(tmpDir, outputPath);

    const zip = new AdmZip(outputPath);
    const entry = zip.getEntry('manifest.yaml');
    expect(entry).not.toBeNull();
    expect(entry!.getData().toString('utf-8')).toBe(content);
  });

  it('excludes files matching ignore patterns', () => {
    writeFile(tmpDir, 'manifest.yaml', 'name: test');
    writeFile(tmpDir, '.DS_Store', '');
    writeFile(tmpDir, '.gitignore', 'build/');
    writeFile(tmpDir, 'build/output.js', 'compiled');
    writeFile(tmpDir, 'src/index.ts', 'export {}');

    const outputPath = path.join(outputDir, 'test.zip');
    createZipPackage(tmpDir, outputPath);

    const zip = new AdmZip(outputPath);
    const entries = zip.getEntries().map((e) => e.entryName);

    expect(entries).toContain('manifest.yaml');
    expect(entries).toContain(path.join('src', 'index.ts'));
    expect(entries).not.toContain('.DS_Store');
    expect(entries).not.toContain('.gitignore');
    expect(entries).not.toContain(path.join('build', 'output.js'));
  });

  it('handles nested directory structures', () => {
    writeFile(tmpDir, 'manifest.yaml', 'name: test');
    writeFile(tmpDir, 'a/b/c/deep.yaml', 'deep');
    writeFile(tmpDir, 'a/b/sibling.yaml', 'sibling');

    const outputPath = path.join(outputDir, 'test.zip');
    createZipPackage(tmpDir, outputPath);

    const zip = new AdmZip(outputPath);
    const entries = zip.getEntries().map((e) => e.entryName);

    expect(entries).toContain(path.join('a', 'b', 'c', 'deep.yaml'));
    expect(entries).toContain(path.join('a', 'b', 'sibling.yaml'));
  });

  describe('{env:...} substitution', () => {
    const env = (values: Record<string, string>) => ({ values, sources: ['test'] });

    it('resolves tokens in the shipped YAML', () => {
      writeFile(tmpDir, 'manifest.yaml', 'name: test\nversion: 1.0.0');
      writeFile(
        tmpDir,
        'content/story/.node.yaml',
        'properties:\n  base_url: "{env:PREVIEW_SERVER}"\n'
      );

      const outputPath = path.join(outputDir, 'test.zip');
      createZipPackage(tmpDir, outputPath, env({ PREVIEW_SERVER: 'https://preview.example.ch' }));

      const zip = new AdmZip(outputPath);
      const entry = zip.getEntry('content/story/.node.yaml');
      expect(entry).not.toBeNull();
      expect(entry!.getData().toString('utf-8')).toBe(
        'properties:\n  base_url: "https://preview.example.ch"\n'
      );
    });

    it('resolves tokens inside post_install inputs in manifest.yaml', () => {
      writeFile(
        tmpDir,
        'manifest.yaml',
        'name: test\nversion: 1.0.0\npost_install:\n' +
          '  - function: /lib/shop/set-stock\n' +
          '    input: { store: "{env:STORE_ID}", qty: "{env:OPENING_STOCK:-50}" }\n'
      );

      const outputPath = path.join(outputDir, 'test.zip');
      createZipPackage(tmpDir, outputPath, env({ STORE_ID: 'main' }));

      const zip = new AdmZip(outputPath);
      expect(zip.getEntry('manifest.yaml')!.getData().toString('utf-8')).toBe(
        'name: test\nversion: 1.0.0\npost_install:\n' +
          '  - function: /lib/shop/set-stock\n' +
          '    input: { store: "main", qty: "50" }\n'
      );
    });

    it('applies inline defaults with no environment at all', () => {
      writeFile(tmpDir, 'manifest.yaml', 'name: test\nversion: 1.0.0');
      writeFile(tmpDir, 'a.yaml', 'url: "{env:PREVIEW_SERVER:-http://localhost:5173}"');

      const outputPath = path.join(outputDir, 'test.zip');
      createZipPackage(tmpDir, outputPath);

      const zip = new AdmZip(outputPath);
      expect(zip.getEntry('a.yaml')!.getData().toString('utf-8')).toBe(
        'url: "http://localhost:5173"'
      );
    });

    it('throws and writes no package when a token cannot be resolved', () => {
      writeFile(tmpDir, 'manifest.yaml', 'name: test\nversion: 1.0.0');
      writeFile(tmpDir, 'content/story/.node.yaml', 'url: "{env:MISSING_VAR}"');

      const outputPath = path.join(outputDir, 'test.zip');

      expect(() => createZipPackage(tmpDir, outputPath)).toThrow(/MISSING_VAR/);
      expect(fs.existsSync(outputPath)).toBe(false);
    });

    it('leaves binary files byte-identical', () => {
      const binary = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x00, 0xff, 0xfe, 0x7b, 0x65]);
      writeFile(tmpDir, 'manifest.yaml', 'name: test\nversion: 1.0.0');
      fs.mkdirSync(path.join(tmpDir, 'static'), { recursive: true });
      fs.writeFileSync(path.join(tmpDir, 'static/logo.png'), binary);

      const outputPath = path.join(outputDir, 'test.zip');
      createZipPackage(tmpDir, outputPath, env({ ANYTHING: 'x' }));

      const zip = new AdmZip(outputPath);
      const entry = zip.getEntry(path.join('static', 'logo.png'));
      expect(entry).not.toBeNull();
      expect(entry!.getData().equals(binary)).toBe(true);
    });

    it('excludes .env files from the package', () => {
      writeFile(tmpDir, 'manifest.yaml', 'name: test\nversion: 1.0.0');
      writeFile(tmpDir, '.env', 'PREVIEW_SERVER=http://localhost:5173');
      writeFile(tmpDir, '.env.production', 'PREVIEW_SERVER=https://preview.example.ch');

      const outputPath = path.join(outputDir, 'test.zip');
      createZipPackage(tmpDir, outputPath);

      const entries = new AdmZip(outputPath).getEntries().map((e) => e.entryName);
      expect(entries).not.toContain('.env');
      expect(entries).not.toContain('.env.production');
    });
  });
});

describe('DEFAULT_IGNORE_PATTERNS', () => {
  it('includes common unwanted patterns', () => {
    expect(DEFAULT_IGNORE_PATTERNS).toContain('.git');
    expect(DEFAULT_IGNORE_PATTERNS).toContain('.DS_Store');
    expect(DEFAULT_IGNORE_PATTERNS).toContain('node_modules');
    expect(DEFAULT_IGNORE_PATTERNS).toContain('*.log');
  });
});

describe('formatInstallFailure', () => {
  const detail = [
    "Package 'site' installed 40 content node(s) but 3 were rejected (1 root cause(s); 2 more only because they reference a rejected entry):",
    '  - stories:/bap/a (studio:Page): Validation failed: Referenced node not found: assets:/x.jpg (at hero.image)',
    'Rejected because they reference a rejected entry:',
    '  - stories:/bap/b (studio:Page): references stories:/bap/a, which was rejected in this install (root cause: ...)',
  ].join('\n');

  it('prints the rejection reasons, not just "failed"', () => {
    const msg = formatInstallFailure('site', detail, 'job-1');
    expect(msg).toContain('Referenced node not found: assets:/x.jpg (at hero.image)');
    expect(msg).toContain('references stories:/bap/a, which was rejected');
  });

  it('says where to look when the server gave no reason', () => {
    expect(formatInstallFailure('site', undefined, 'job-1')).toMatch(/no reason for job job-1.*content entry rejected/);
  });

  it('cuts a very long list and says how much was left out', () => {
    const long = ['head', ...Array.from({ length: 100 }, (_, i) => `  - entry ${i}`)].join('\n');
    const msg = formatInstallFailure('site', long);
    expect(msg).toContain('  - entry 58');
    expect(msg).not.toContain('  - entry 59\n');
    expect(msg).toContain('41 more line(s)');
  });
});

describe('installVerdict', () => {
  const pkg = (p: Partial<PackageSummary>): PackageSummary => ({
    id: '1',
    name: 'site',
    version: '1.0.0',
    installed: true,
    status: 'installed',
    ...p,
  });

  it('keeps waiting while the job runs, even when the node already says installed', () => {
    // The node flips to `installed` BEFORE the post_install hooks run.
    for (const status of ['Scheduled', 'Running', 'Executing']) {
      expect(installVerdict({ status }, pkg({ postInstallHooks: 2 }))).toEqual({ done: false });
    }
  });

  it('succeeds when the job completes', () => {
    expect(installVerdict({ status: 'Completed' }, pkg({}))).toEqual({ done: true, ok: true });
  });

  it('fails when the job fails, with the node error first', () => {
    expect(
      installVerdict({ status: 'Failed', error: 'job detail' }, pkg({ status: 'failed', error: 'node detail' }))
    ).toEqual({ done: true, ok: false, detail: 'node detail' });
    expect(installVerdict({ status: 'Failed', error: 'job detail' }, undefined)).toEqual({
      done: true,
      ok: false,
      detail: 'job detail',
    });
    expect(installVerdict({ status: 'Cancelled' }, pkg({}))).toMatchObject({ done: true, ok: false });
  });

  describe('without a job answer (old server, purged job, no job id)', () => {
    it('waits for the hook results when the manifest declares hooks', () => {
      expect(installVerdict(null, pkg({ postInstallHooks: 1 }))).toEqual({ done: false });
      expect(
        installVerdict(
          null,
          pkg({ postInstallHooks: 1, postInstallResults: [{ function: '/lib/a', status: 'completed' }] })
        )
      ).toEqual({ done: true, ok: true });
    });

    it('fails on a failed hook or a failed node', () => {
      expect(
        installVerdict(
          null,
          pkg({ postInstallHooks: 1, postInstallResults: [{ function: '/lib/a', status: 'failed' }] })
        )
      ).toMatchObject({ done: true, ok: false });
      expect(installVerdict(null, pkg({ status: 'failed', error: 'x' }))).toEqual({
        done: true,
        ok: false,
        detail: 'x',
      });
    });

    it('is done at installed when there are no hooks, and waits otherwise', () => {
      expect(installVerdict(null, pkg({}))).toEqual({ done: true, ok: true });
      expect(installVerdict(null, pkg({ status: 'installing', installed: false }))).toEqual({ done: false });
      expect(installVerdict(null, undefined)).toEqual({ done: false });
    });
  });

  it('allows the whole hook budget (1200 s) and then some', () => {
    expect(INSTALL_POLL_TIMEOUT_MS).toBeGreaterThan(1_200_000);
  });
});

describe('formatPostInstallResults', () => {
  it('prints one line per hook with its status, duration and error', () => {
    expect(
      formatPostInstallResults([
        { function: '/lib/a', status: 'completed', duration_ms: 12 },
        { function: '/lib/b', status: 'failed', duration_ms: 3, error: 'boom' },
        { function: '/lib/c', status: 'skipped', error: 'an earlier hook failed' },
      ])
    ).toEqual([
      '  post_install /lib/a: completed (12 ms)',
      '  post_install /lib/b: failed (3 ms): boom',
      '  post_install /lib/c: skipped: an earlier hook failed',
    ]);
    expect(formatPostInstallResults(undefined)).toEqual([]);
  });
});

describe('countPostInstallHooks', () => {
  it('reads the JSON string the server stores, or a plain list', () => {
    expect(countPostInstallHooks('[{"function":"/lib/a"},{"function":"/lib/b"}]')).toBe(2);
    expect(countPostInstallHooks([{ function: '/lib/a' }])).toBe(1);
    expect(countPostInstallHooks(undefined)).toBeUndefined();
    expect(countPostInstallHooks('not json')).toBeUndefined();
  });
});

describe('a failed attempt the engine retries', () => {
  const failedNode: PackageSummary = {
    id: '1',
    name: 'site',
    version: '1.0.0',
    installed: false,
    status: 'failed',
    error: "Package 'site' installed 369 content node(s) but 28 were rejected",
  };
  const retrying = {
    status: 'Scheduled',
    error: 'installed 369 content node(s) but 28 were rejected',
    retryCount: 1,
    maxRetries: 3,
    nextRetryAt: '2026-11-17T12:00:10Z',
  };

  it('is not final while the retry is pending, although the node says failed', () => {
    expect(installVerdict(retrying, failedNode)).toEqual({ done: false });
  });

  it('is a success when the retry completes', () => {
    expect(
      installVerdict({ ...retrying, status: 'Completed' }, { ...failedNode, status: 'installed', installed: true })
    ).toEqual({ done: true, ok: true });
  });

  it('is a failure once the engine gives up', () => {
    expect(installVerdict({ ...retrying, status: 'Failed', retryCount: 3 }, failedNode)).toMatchObject({
      done: true,
      ok: false,
    });
  });

  it('says a retry is pending, and stays quiet otherwise', () => {
    expect(retryNotice(retrying)).toBe(
      '  attempt failed: installed 369 content node(s) but 28 were rejected; the engine retries (retry 1/3) at 2026-11-17T12:00:10Z'
    );
    expect(retryNotice({ status: 'Scheduled', retryCount: 0 })).toBeNull();
    expect(retryNotice({ status: 'Executing', retryCount: 1 })).toBeNull();
    expect(retryNotice(null)).toBeNull();
  });
});

describe('results of this run only', () => {
  const rows = [
    { function: '/lib/a', status: 'completed', job_id: 'old-job' },
    { function: '/lib/a', status: 'completed', job_id: 'new-job' },
    { function: '/lib/b', status: 'completed' },
  ];

  it('drops rows stamped with another job and keeps unstamped ones', () => {
    expect(currentResults(rows, 'new-job').map((r) => r.job_id)).toEqual(['new-job', undefined]);
    expect(currentResults(rows, undefined)).toHaveLength(3);
    expect(currentResults(undefined, 'x')).toEqual([]);
  });

  it('does not take an earlier run\'s results as this run\'s hooks having run', () => {
    const pkg: PackageSummary = {
      id: '1',
      name: 'site',
      version: '1.0.0',
      installed: true,
      status: 'installed',
      postInstallHooks: 1,
      postInstallResults: [{ function: '/lib/a', status: 'completed', job_id: 'old-job' }],
    };
    expect(installVerdict(null, pkg, 'new-job')).toEqual({ done: false });
    expect(installVerdict(null, pkg, 'old-job')).toEqual({ done: true, ok: true });
  });
});

describe('hook warnings and output', () => {
  it('prints each warning and the output under its hook', () => {
    expect(
      formatPostInstallResults([
        {
          function: '/lib/seed',
          status: 'completed',
          duration_ms: 40,
          warnings: ['self-registration closed', 'audiences:/speakers deferred'],
          output: '{"seeded":3}',
        },
      ])
    ).toEqual([
      '  post_install /lib/seed: completed (40 ms)',
      '    warning: self-registration closed',
      '    warning: audiences:/speakers deferred',
      '    output: {"seeded":3}',
    ]);
  });
});
