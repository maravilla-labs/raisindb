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
} from './package.js';

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
