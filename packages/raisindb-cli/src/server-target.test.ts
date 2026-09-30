import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import fs from 'fs';
import os from 'os';
import path from 'path';
import { Command } from 'commander';
import {
  getServer,
  normalizeServerUrl,
  pinServerFromOptions,
  sameServer,
  setServerOverride,
} from './config.js';
import { getToken, resetTokenWarnings } from './auth.js';
import { getBaseUrl, getHeaders, installPackage, listPackages } from './api.js';

/**
 * Which server a command talks to, and which token it sends there.
 *
 * vitest.setup.ts gives every test file a temp HOME + cwd, so the .raisinrc
 * written here is the only one loadConfig() can find. fetch is stubbed: no
 * test makes a real request.
 */

const PROD = 'https://prod.example.com';
const LOCAL = 'http://localhost:8080';

function rcPath(): string {
  return path.join(os.homedir(), '.raisinrc');
}

function writeRc(server: string | null, token: string | null): void {
  const lines = [];
  if (server) lines.push(`server: ${server}`);
  if (token) lines.push(`token: ${token}`);
  fs.writeFileSync(rcPath(), lines.join('\n') + '\n', 'utf-8');
}

interface Recorded {
  url: string;
  method: string;
  authorization: string | null;
}

function stubFetch(respond: (url: string, method: string) => unknown = () => ({})) {
  const calls: Recorded[] = [];
  const fetchMock = vi.fn(async (input: unknown, init?: RequestInit) => {
    const url = String(input);
    const method = (init?.method ?? 'GET').toUpperCase();
    const headers = new Headers(init?.headers as ConstructorParameters<typeof Headers>[0]);
    calls.push({ url, method, authorization: headers.get('authorization') });
    return new Response(JSON.stringify(respond(url, method)), {
      status: 200,
      headers: { 'Content-Type': 'application/json' },
    });
  });
  vi.stubGlobal('fetch', fetchMock);
  return calls;
}

let stderr: ReturnType<typeof vi.spyOn>;

beforeEach(() => {
  expect(os.homedir()).toContain('raisindb-cli-home-'); // never the real HOME
  setServerOverride(null);
  resetTokenWarnings();
  delete process.env.RAISINDB_SERVER;
  delete process.env.RAISINDB_TOKEN;
  if (fs.existsSync(rcPath())) fs.rmSync(rcPath());
  stderr = vi.spyOn(console, 'error').mockImplementation(() => {});
});

afterEach(() => {
  setServerOverride(null);
  delete process.env.RAISINDB_SERVER;
  delete process.env.RAISINDB_TOKEN;
  vi.unstubAllGlobals();
  stderr.mockRestore();
});

describe('server resolution', () => {
  it('--server wins over the .raisinrc server', () => {
    writeRc(PROD, 'prod-token');
    pinServerFromOptions({ server: LOCAL });
    expect(getServer()).toBe(LOCAL);
    expect(getBaseUrl()).toBe(LOCAL);
  });

  it('--server is used when nothing else names a server', () => {
    pinServerFromOptions({ server: LOCAL });
    expect(getBaseUrl()).toBe(LOCAL);
  });

  it('--server wins over RAISINDB_SERVER', () => {
    process.env.RAISINDB_SERVER = PROD;
    pinServerFromOptions({ server: LOCAL });
    expect(getBaseUrl()).toBe(LOCAL);
  });

  it('RAISINDB_SERVER still wins over .raisinrc when there is no --server', () => {
    writeRc(PROD, 'prod-token');
    process.env.RAISINDB_SERVER = LOCAL;
    pinServerFromOptions({});
    expect(getBaseUrl()).toBe(LOCAL);
  });

  it('falls back to .raisinrc, then the local default', () => {
    expect(getBaseUrl()).toBe('http://localhost:8081');
    writeRc(PROD, 'prod-token');
    expect(getBaseUrl()).toBe(PROD);
  });

  it('a bare `function test --server` switch (boolean) pins nothing', () => {
    writeRc(PROD, 'prod-token');
    pinServerFromOptions({ server: true });
    expect(getBaseUrl()).toBe(PROD);
  });

  it('the preAction hook pins -s/--server for a subcommand before its action runs', async () => {
    writeRc(PROD, 'prod-token');
    let seen = '';
    const program = new Command()
      .exitOverride()
      .hook('preAction', (_p, action) => pinServerFromOptions(action.opts()));
    program
      .command('deploy <folder>')
      .option('-s, --server <url>')
      .action(() => {
        seen = getBaseUrl();
      });
    await program.parseAsync(['node', 'raisindb', 'deploy', './pkg', '-s', LOCAL]);
    expect(seen).toBe(LOCAL);
  });
});

describe('normalizeServerUrl / sameServer', () => {
  it('ignores case, default ports, trailing slashes and raisin:// schemes', () => {
    expect(normalizeServerUrl('HTTP://LocalHost:8080/')).toBe('http://localhost:8080');
    expect(sameServer('https://prod.example.com:443/', 'https://prod.example.com')).toBe(true);
    expect(sameServer('http://h:80', 'http://h')).toBe(true);
    expect(sameServer('raisins://prod.example.com', 'https://prod.example.com')).toBe(true);
    expect(sameServer('localhost:8080', 'http://localhost:8080')).toBe(true);
  });

  it('tells different servers apart', () => {
    expect(sameServer('http://localhost:8080', 'http://localhost:8081')).toBe(false);
    expect(sameServer('http://prod.example.com', 'https://prod.example.com')).toBe(false);
    expect(sameServer(LOCAL, PROD)).toBe(false);
    expect(sameServer(LOCAL, null)).toBe(false);
  });
});

describe('token scoping', () => {
  it('sends no .raisinrc token to a different server, and says why once', () => {
    writeRc(PROD, 'prod-token');
    setServerOverride(LOCAL);
    expect(getToken()).toBeNull();
    expect(getHeaders().Authorization).toBeUndefined();
    expect(stderr).toHaveBeenCalledTimes(1);
    const hint = String(stderr.mock.calls[0][0]);
    expect(hint).toContain(`raisindb login -s ${LOCAL}`);
    expect(hint).toContain('RAISINDB_TOKEN');
    expect(hint).not.toContain('prod-token');
  });

  it('sends the .raisinrc token when the target is the server it was issued by', () => {
    writeRc(`${PROD}/`, 'prod-token');
    setServerOverride('HTTPS://prod.example.com:443');
    expect(getToken()).toBe('prod-token');
    expect(getHeaders().Authorization).toBe('Bearer prod-token');
    expect(stderr).not.toHaveBeenCalled();
  });

  it('keeps RAISINDB_TOKEN as an explicit override for any server', () => {
    writeRc(PROD, 'prod-token');
    process.env.RAISINDB_TOKEN = 'env-token';
    setServerOverride(LOCAL);
    expect(getToken()).toBe('env-token');
  });

  it('does not send a token that has no server stored with it', () => {
    writeRc(null, 'orphan-token');
    expect(getToken(LOCAL)).toBeNull();
  });
});

describe('HTTP calls follow --server', () => {
  it('sends requests to the flag server without the other server\'s token', async () => {
    writeRc(PROD, 'prod-token');
    setServerOverride(LOCAL);
    const calls = stubFetch((_url, method) =>
      method === 'POST' ? { package_name: 'p', version: '1', installed: false, job_id: 'j' } : []
    );

    await installPackage('studio', 'my-pkg', 'main', 'sync');
    await listPackages('studio');

    expect(calls).toHaveLength(2);
    for (const call of calls) {
      expect(call.url.startsWith(`${LOCAL}/`)).toBe(true);
      expect(call.url).not.toContain('prod.example.com');
      expect(call.authorization).toBeNull();
    }
  });

  it('sends the token when the flag server is the logged-in server', async () => {
    writeRc(LOCAL, 'local-token');
    setServerOverride(`${LOCAL}/`);
    const calls = stubFetch(() => []);

    await listPackages('studio');

    expect(calls[0].url).toBe(`${LOCAL}/api/repos/studio/packages`);
    expect(calls[0].authorization).toBe('Bearer local-token');
  });
});
