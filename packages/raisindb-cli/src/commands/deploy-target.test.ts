import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import fs from 'fs';
import os from 'os';
import path from 'path';

/**
 * `raisindb deploy <dir> --server <url> --install` must send EVERY request —
 * upload, install, status polling — to <url>, even with a login to another
 * server in ~/.raisinrc. It used to go to the .raisinrc server: a deploy
 * aimed at a local server landed on production.
 *
 * Only the .rap build is mocked (it needs the schema WASM); upload, install
 * and polling run the real code against a stubbed fetch. No real request is
 * made. HOME and cwd are temp dirs (vitest.setup.ts).
 */
vi.mock('./package.js', async (importOriginal) => {
  const actual = await importOriginal<typeof import('./package.js')>();
  return {
    ...actual,
    createPackage: vi.fn(async (_folder: string, output: string) => {
      fs.writeFileSync(output, 'rap');
    }),
  };
});

import { deployPackage } from './deploy.js';
import { setServerOverride } from '../config.js';
import { resetTokenWarnings } from '../auth.js';

const PROD = 'https://prod.example.com';
const LOCAL = 'http://localhost:8080';

interface Recorded {
  url: string;
  method: string;
  authorization: string | null;
}

describe('deploy --server', () => {
  let pkgDir: string;
  let calls: Recorded[];
  let stderrLines: string[];

  beforeEach(() => {
    expect(os.homedir()).toContain('raisindb-cli-home-'); // never the real HOME
    setServerOverride(null);
    resetTokenWarnings();
    delete process.env.RAISINDB_SERVER;
    delete process.env.RAISINDB_TOKEN;

    pkgDir = fs.mkdtempSync(path.join(os.tmpdir(), 'raisindb-deploy-target-'));
    fs.writeFileSync(path.join(pkgDir, 'manifest.yaml'), 'name: demo\nversion: 1.0.0\n');

    calls = [];
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: unknown, init?: RequestInit) => {
        const url = String(input);
        const method = (init?.method ?? 'GET').toUpperCase();
        calls.push({
          url,
          method,
          authorization: new Headers(init?.headers as ConstructorParameters<typeof Headers>[0]).get('authorization'),
        });
        let body: unknown = {};
        if (method === 'POST' && url.includes('/head/packages/')) {
          body = { storedKey: 'k', url: 'u' }; // small upload, no background job
        } else if (method === 'POST' && url.includes('/install')) {
          body = { package_name: 'demo', version: '1.0.0', installed: false, job_id: 'job-1' };
        } else if (method === 'GET' && url.endsWith('/packages')) {
          body = [{ id: '1', name: 'demo', properties: { version: '1.0.0', installed: true, status: 'installed' } }];
        }
        return new Response(JSON.stringify(body), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        });
      })
    );

    stderrLines = [];
    vi.spyOn(console, 'error').mockImplementation((...args: unknown[]) => {
      stderrLines.push(args.map(String).join(' '));
    });
    vi.spyOn(console, 'log').mockImplementation(() => {});
  });

  afterEach(() => {
    setServerOverride(null);
    delete process.env.RAISINDB_TOKEN;
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    fs.rmSync(pkgDir, { recursive: true, force: true });
    const rc = path.join(os.homedir(), '.raisinrc');
    if (fs.existsSync(rc)) fs.rmSync(rc);
  });

  it('uploads and installs on the --server target, not the .raisinrc login', async () => {
    fs.writeFileSync(path.join(os.homedir(), '.raisinrc'), `server: ${PROD}\ntoken: prod-token\n`);
    process.env.RAISINDB_TOKEN = 'local-token'; // the credential for LOCAL

    await deployPackage(pkgDir, { server: LOCAL, repo: 'studio', install: true });

    const methods = calls.map((c) => c.method);
    expect(methods).toContain('POST'); // upload + install happened
    expect(calls.some((c) => c.url.includes('/install'))).toBe(true);
    for (const call of calls) {
      expect(call.url.startsWith(`${LOCAL}/`)).toBe(true);
      expect(call.authorization).toBe('Bearer local-token');
    }
    // The target was announced before the write.
    expect(stderrLines).toContain(`→ ${LOCAL} (repo studio, branch main)`);
  }, 20_000);

  it('refuses before building when there is no credential for the --server target', async () => {
    fs.writeFileSync(path.join(os.homedir(), '.raisinrc'), `server: ${PROD}\ntoken: prod-token\n`);

    await expect(deployPackage(pkgDir, { server: LOCAL, repo: 'studio', install: true })).rejects.toThrow(
      `raisindb login -s ${LOCAL}`
    );
    expect(calls).toHaveLength(0); // the production token went nowhere
    expect(stderrLines.join('\n')).not.toContain('prod-token');
  });
});
