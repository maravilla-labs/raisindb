import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import fs from 'fs';
import path from 'path';
import os from 'os';
import { getServer, getDefaultRepo } from './config.js';
import { getToken } from './auth.js';

/**
 * Env-var precedence tests for non-interactive (CI) usage:
 *   RAISINDB_SERVER > .raisinrc server
 *   RAISINDB_TOKEN  > .raisinrc token
 *   RAISINDB_REPO   > .raisinrc default_repo
 *
 * loadConfig() searches up the directory tree from process.cwd() for a
 * .raisinrc file, so we chdir into a temp dir with a known config file.
 */

const ENV_KEYS = ['RAISINDB_SERVER', 'RAISINDB_TOKEN', 'RAISINDB_REPO'] as const;

describe('env-var precedence (RAISINDB_SERVER / RAISINDB_TOKEN / RAISINDB_REPO)', () => {
  let tmpDir: string;
  let originalCwd: string;
  const savedEnv: Record<string, string | undefined> = {};

  beforeEach(() => {
    originalCwd = process.cwd();
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'raisindb-cli-config-test-'));
    fs.writeFileSync(
      path.join(tmpDir, '.raisinrc'),
      [
        'server: http://config-file-server:1111',
        'token: config-file-token',
        'default_repo: config-file-repo',
      ].join('\n'),
      'utf-8'
    );
    process.chdir(tmpDir);
    for (const key of ENV_KEYS) {
      savedEnv[key] = process.env[key];
      delete process.env[key];
    }
  });

  afterEach(() => {
    process.chdir(originalCwd);
    fs.rmSync(tmpDir, { recursive: true, force: true });
    for (const key of ENV_KEYS) {
      if (savedEnv[key] === undefined) {
        delete process.env[key];
      } else {
        process.env[key] = savedEnv[key];
      }
    }
  });

  describe('getServer', () => {
    it('falls back to the config file when no env var is set', () => {
      expect(getServer()).toBe('http://config-file-server:1111');
    });

    it('prefers RAISINDB_SERVER over the config file', () => {
      process.env.RAISINDB_SERVER = 'http://env-server:2222';
      expect(getServer()).toBe('http://env-server:2222');
    });

    it('ignores an empty RAISINDB_SERVER', () => {
      process.env.RAISINDB_SERVER = '   ';
      expect(getServer()).toBe('http://config-file-server:1111');
    });
  });

  describe('getToken', () => {
    it('falls back to the config file when no env var is set', () => {
      expect(getToken()).toBe('config-file-token');
    });

    it('prefers RAISINDB_TOKEN over the config file', () => {
      process.env.RAISINDB_TOKEN = 'env-token';
      expect(getToken()).toBe('env-token');
    });

    it('ignores an empty RAISINDB_TOKEN', () => {
      process.env.RAISINDB_TOKEN = '';
      expect(getToken()).toBe('config-file-token');
    });
  });

  describe('getDefaultRepo', () => {
    it('falls back to the config file when no env var is set', () => {
      expect(getDefaultRepo()).toBe('config-file-repo');
    });

    it('prefers RAISINDB_REPO over the config file', () => {
      process.env.RAISINDB_REPO = 'env-repo';
      expect(getDefaultRepo()).toBe('env-repo');
    });

    it('trims RAISINDB_REPO', () => {
      process.env.RAISINDB_REPO = '  env-repo  ';
      expect(getDefaultRepo()).toBe('env-repo');
    });
  });
});
