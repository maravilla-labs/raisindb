import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import fs from 'fs';
import os from 'os';
import path from 'path';
import yaml from 'yaml';
import { loadSyncConfig, saveSyncConfig, SyncConfig } from './config.js';

describe('sync config {env:...} substitution', () => {
  let tmpDir: string;
  let savedEnv: NodeJS.ProcessEnv;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'raisindb-sync-config-'));
    savedEnv = { ...process.env };
    delete process.env.RAISIN_SERVER;
    delete process.env.RAISIN_BRANCH;
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
    process.env = savedEnv;
    vi.restoreAllMocks();
  });

  const writeConfig = (content: string) =>
    fs.writeFileSync(path.join(tmpDir, '.raisindb-cli.yaml'), content, 'utf-8');

  it('resolves tokens from the process environment', () => {
    process.env.RAISIN_SERVER = 'https://db.example.ch';
    writeConfig(
      'version: 1\nserver: "{env:RAISIN_SERVER}"\nrepository: studio\nbranch: main\n'
    );

    const config = loadSyncConfig(tmpDir);
    expect(config?.server).toBe('https://db.example.ch');
  });

  it('resolves tokens from a .env file next to the config', () => {
    fs.writeFileSync(path.join(tmpDir, '.env'), 'RAISIN_SERVER=http://localhost:8080\n');
    writeConfig('version: 1\nserver: "{env:RAISIN_SERVER}"\nrepository: studio\n');

    expect(loadSyncConfig(tmpDir)?.server).toBe('http://localhost:8080');
  });

  it('selects a profile file', () => {
    fs.writeFileSync(path.join(tmpDir, '.env'), 'RAISIN_SERVER=http://localhost:8080\n');
    fs.writeFileSync(
      path.join(tmpDir, '.env.production'),
      'RAISIN_SERVER=https://db.example.ch\n'
    );
    writeConfig('version: 1\nserver: "{env:RAISIN_SERVER}"\nrepository: studio\n');

    expect(loadSyncConfig(tmpDir, { profile: 'production' })?.server).toBe(
      'https://db.example.ch'
    );
  });

  it('applies inline defaults', () => {
    writeConfig(
      'version: 1\nserver: "{env:RAISIN_SERVER:-http://localhost:8080}"\nrepository: studio\n'
    );

    expect(loadSyncConfig(tmpDir)?.server).toBe('http://localhost:8080');
  });

  it('returns null and explains when a token cannot be resolved', () => {
    const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
    writeConfig('version: 1\nserver: "{env:RAISIN_SERVER}"\nrepository: studio\n');

    expect(loadSyncConfig(tmpDir)).toBeNull();
    expect(errorSpy).toHaveBeenCalledWith(expect.stringContaining('{env:RAISIN_SERVER}'));
  });

  it('ignores .env files when syncing', () => {
    writeConfig('version: 1\nserver: http://localhost:8080\nrepository: studio\n');
    const config = loadSyncConfig(tmpDir);
    expect(config?.ignore).toContain('.env');
    expect(config?.ignore).toContain('.env.*');
  });
});

describe('saveSyncConfig', () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'raisindb-sync-save-'));
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
    vi.restoreAllMocks();
  });

  const config: SyncConfig = {
    version: 1,
    server: 'http://localhost:8080',
    repository: 'studio',
    branch: 'main',
    remote_path: '/',
    conflict_strategy: 'prompt',
    ignore: [],
  };

  it('writes a config when none exists', () => {
    saveSyncConfig(tmpDir, config);
    const written = yaml.parse(
      fs.readFileSync(path.join(tmpDir, '.raisindb-cli.yaml'), 'utf-8')
    );
    expect(written.server).toBe('http://localhost:8080');
  });

  it('refuses to overwrite a config that uses {env:...} tokens', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const original = 'version: 1\nserver: "{env:RAISIN_SERVER}"\nrepository: studio\n';
    fs.writeFileSync(path.join(tmpDir, '.raisindb-cli.yaml'), original, 'utf-8');

    saveSyncConfig(tmpDir, config);

    // The tokens survive — resolved values were NOT written back.
    expect(fs.readFileSync(path.join(tmpDir, '.raisindb-cli.yaml'), 'utf-8')).toBe(original);
    expect(warnSpy).toHaveBeenCalledWith(expect.stringContaining('{env:...}'));
  });
});

describe('legacy .raisin-sync.yaml', () => {
  // The rename exists because this filename is ALSO a package's install
  // policy. These pin the discriminator, because "renamed away" is not the
  // same as "cannot happen again".
  let tmpDir: string;
  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'raisindb-legacy-'));
  });
  afterEach(() => fs.rmSync(tmpDir, { recursive: true, force: true }));

  it('still reads a legacy file that IS a CLI config', () => {
    fs.writeFileSync(
      path.join(tmpDir, '.raisin-sync.yaml'),
      'server: http://localhost:8080\nrepo: studio\n'
    );
    const cfg = loadSyncConfig(tmpDir);
    expect(cfg?.server).toBe('http://localhost:8080');
  });

  it('IGNORES a legacy file that is a package install policy', () => {
    // This exact shape used to be loaded as a CLI config, leaving `server`
    // undefined and throwing from toHttpUrl on push — while --dry-run passed.
    fs.writeFileSync(
      path.join(tmpDir, '.raisin-sync.yaml'),
      'defaults:\n  mode: skip\nfilters:\n  - path: /functions\n    mode: replace\n'
    );
    expect(loadSyncConfig(tmpDir)).toBeNull();
  });

  it('IGNORES an empty legacy file', () => {
    fs.writeFileSync(path.join(tmpDir, '.raisin-sync.yaml'), '');
    expect(loadSyncConfig(tmpDir)).toBeNull();
  });

  it('prefers the new name when both exist', () => {
    fs.writeFileSync(path.join(tmpDir, '.raisin-sync.yaml'), 'server: http://legacy\n');
    fs.writeFileSync(path.join(tmpDir, '.raisindb-cli.yaml'), 'server: http://current\n');
    expect(loadSyncConfig(tmpDir)?.server).toBe('http://current');
  });
});
