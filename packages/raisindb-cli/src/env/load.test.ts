import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import fs from 'fs';
import os from 'os';
import path from 'path';
import { assertEnvFilesExist, isSubstitutableFile, loadEnvContext } from './load.js';

describe('env loading', () => {
  let tmpDir: string;
  let savedEnv: NodeJS.ProcessEnv;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'raisindb-env-'));
    savedEnv = { ...process.env };
    // These names are exercised below; make sure a stray real value can't leak in.
    delete process.env.PREVIEW_SERVER;
    delete process.env.RAISIN_BRANCH;
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
    process.env = savedEnv;
  });

  const write = (name: string, content: string) =>
    fs.writeFileSync(path.join(tmpDir, name), content, 'utf-8');

  it('returns process env when no files exist', () => {
    process.env.PREVIEW_SERVER = 'https://from-process';
    const env = loadEnvContext(tmpDir);
    expect(env.values.PREVIEW_SERVER).toBe('https://from-process');
    expect(env.sources).toEqual(['process environment']);
  });

  it('reads .env and records it as a source', () => {
    write('.env', 'PREVIEW_SERVER=http://localhost:5173\n');
    const env = loadEnvContext(tmpDir);
    expect(env.values.PREVIEW_SERVER).toBe('http://localhost:5173');
    expect(env.sources).toContain(path.join(tmpDir, '.env'));
  });

  it('lets the profile file override .env', () => {
    write('.env', 'PREVIEW_SERVER=http://localhost:5173\n');
    write('.env.production', 'PREVIEW_SERVER=https://preview.example.ch\n');

    expect(loadEnvContext(tmpDir).values.PREVIEW_SERVER).toBe('http://localhost:5173');
    expect(loadEnvContext(tmpDir, { profile: 'production' }).values.PREVIEW_SERVER).toBe(
      'https://preview.example.ch'
    );
  });

  it('lets .env.local override the profile file', () => {
    write('.env', 'PREVIEW_SERVER=a\n');
    write('.env.production', 'PREVIEW_SERVER=b\n');
    write('.env.local', 'PREVIEW_SERVER=c\n');

    expect(loadEnvContext(tmpDir, { profile: 'production' }).values.PREVIEW_SERVER).toBe('c');
  });

  it('lets .env.<profile>.local win over .env.local', () => {
    write('.env.local', 'PREVIEW_SERVER=c\n');
    write('.env.production.local', 'PREVIEW_SERVER=d\n');

    expect(loadEnvContext(tmpDir, { profile: 'production' }).values.PREVIEW_SERVER).toBe('d');
  });

  it('applies --env-file after the conventional files, in order', () => {
    write('.env', 'PREVIEW_SERVER=a\n');
    const first = path.join(tmpDir, 'first.env');
    const second = path.join(tmpDir, 'second.env');
    fs.writeFileSync(first, 'PREVIEW_SERVER=b\n');
    fs.writeFileSync(second, 'PREVIEW_SERVER=c\n');

    const env = loadEnvContext(tmpDir, { envFiles: [first, second] });
    expect(env.values.PREVIEW_SERVER).toBe('c');
  });

  it('lets the process environment win over every file', () => {
    write('.env', 'PREVIEW_SERVER=from-file\n');
    write('.env.local', 'PREVIEW_SERVER=from-local\n');
    process.env.PREVIEW_SERVER = 'from-process';

    expect(loadEnvContext(tmpDir).values.PREVIEW_SERVER).toBe('from-process');
  });

  it('merges distinct keys across files rather than replacing them', () => {
    write('.env', 'PREVIEW_SERVER=a\nRAISIN_BRANCH=main\n');
    write('.env.local', 'PREVIEW_SERVER=b\n');

    const env = loadEnvContext(tmpDir);
    expect(env.values.PREVIEW_SERVER).toBe('b');
    expect(env.values.RAISIN_BRANCH).toBe('main');
  });

  it('ignores a missing profile file', () => {
    write('.env', 'PREVIEW_SERVER=a\n');
    expect(loadEnvContext(tmpDir, { profile: 'staging' }).values.PREVIEW_SERVER).toBe('a');
  });
});

describe('assertEnvFilesExist', () => {
  it('throws for an explicitly requested file that is missing', () => {
    expect(() => assertEnvFilesExist(['/nope/does-not-exist.env'])).toThrow(
      /Env file not found/
    );
  });

  it('accepts an empty list', () => {
    expect(() => assertEnvFilesExist()).not.toThrow();
  });
});

describe('isSubstitutableFile', () => {
  it('accepts text formats and rejects binaries', () => {
    expect(isSubstitutableFile('content/story/.node.yaml')).toBe(true);
    expect(isSubstitutableFile('manifest.YML')).toBe(true);
    expect(isSubstitutableFile('functions/handler.js')).toBe(true);
    expect(isSubstitutableFile('static/logo.png')).toBe(false);
    expect(isSubstitutableFile('static/font.woff2')).toBe(false);
  });
});
