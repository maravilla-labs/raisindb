import fs from 'fs';
import path from 'path';
import os from 'os';
import crypto from 'crypto';
import { pipeline } from 'stream/promises';
import { Writable } from 'stream';
import { spawn, type ChildProcess } from 'child_process';
import { createWriteStream } from 'fs';
import React from 'react';
import { render } from 'ink';
import { ServerInstallUI, ServerStartUI, type InstallState } from '../components/ServerInstall.js';

const BIN_NAME = 'raisindb';
const REPO = process.env.RAISINDB_REPO || 'maravilla-labs/raisindb';
const GH_TOKEN = process.env.RAISINDB_GH_TOKEN || process.env.GITHUB_TOKEN || '';
const HTTP_TIMEOUT_MS = Number(process.env.RAISINDB_HTTP_TIMEOUT_MS || 15000);
const HTTP_RETRIES = Math.max(1, Number(process.env.RAISINDB_HTTP_RETRIES || 3));

function getBinDir(): string {
  const dir = path.join(os.homedir(), '.raisindb', 'bin');
  fs.mkdirSync(dir, { recursive: true });
  return dir;
}

function getExecutableName(): string {
  return process.platform === 'win32' ? `${BIN_NAME}.exe` : BIN_NAME;
}

function getInstallPath(): string {
  return path.join(getBinDir(), getExecutableName());
}

function resolveTarget(): { target: string; ext: string } {
  const p = process.platform;
  const a = process.arch;
  if (p === 'linux' && a === 'x64') return { target: 'x86_64-unknown-linux-gnu', ext: 'tar.gz' };
  if (p === 'darwin' && a === 'x64') return { target: 'x86_64-apple-darwin', ext: 'tar.gz' };
  if (p === 'darwin' && a === 'arm64') return { target: 'aarch64-apple-darwin', ext: 'tar.gz' };
  if (p === 'win32' && a === 'x64') return { target: 'x86_64-pc-windows-msvc', ext: 'zip' };
  throw new Error(`Unsupported platform/arch: ${p}/${a}. Build from source or check https://github.com/${REPO}/releases`);
}

function makeFetchHeaders(): Record<string, string> {
  const headers: Record<string, string> = { 'User-Agent': 'raisindb-cli-installer' };
  if (GH_TOKEN) headers['Authorization'] = `Bearer ${GH_TOKEN}`;
  return headers;
}

async function sleep(ms: number): Promise<void> {
  return new Promise(r => setTimeout(r, ms));
}

async function fetchWithTimeout(url: string, opts: RequestInit = {}, timeoutMs = 30000): Promise<Response> {
  const controller = new AbortController();
  const t = setTimeout(() => controller.abort(), timeoutMs);
  try {
    return await fetch(url, { ...opts, signal: controller.signal });
  } finally {
    clearTimeout(t);
  }
}

async function withRetries<T>(fn: () => Promise<T>, tries = HTTP_RETRIES): Promise<T> {
  let lastErr: unknown;
  for (let attempt = 0; attempt < tries; attempt++) {
    try {
      return await fn();
    } catch (e) {
      lastErr = e;
      if (attempt + 1 < tries) {
        await sleep(1000 * Math.pow(2, attempt));
      }
    }
  }
  throw lastErr;
}

async function getLatestReleaseTag(): Promise<string> {
  const version = process.env.RAISINDB_VERSION;
  if (version && version !== 'latest') return version;

  const headers = makeFetchHeaders();
  const api = `https://api.github.com/repos/${REPO}/releases/latest`;

  try {
    const res = await withRetries(() => fetchWithTimeout(api, { headers }, HTTP_TIMEOUT_MS));
    if (!res.ok) throw new Error(`GitHub API ${res.status}`);
    const json = await res.json() as { tag_name?: string };
    if (!json.tag_name) throw new Error('No tag_name in response');
    return json.tag_name;
  } catch {
    const html = `https://github.com/${REPO}/releases/latest`;
    const res = await fetchWithTimeout(html, { headers, redirect: 'manual' }, 10000);
    const loc = res.headers.get('location') || '';
    const m = loc.match(/\/releases\/tag\/([^/]+)$/);
    if (m) return m[1];
    throw new Error('Could not resolve latest release tag');
  }
}

async function currentVersion(): Promise<string | null> {
  const installPath = getInstallPath();
  if (!fs.existsSync(installPath)) return null;

  return new Promise((resolve) => {
    const child = spawn(installPath, ['--version'], { stdio: ['ignore', 'pipe', 'ignore'] });
    let out = '';
    child.stdout.on('data', (d: Buffer) => (out += d.toString()));
    child.on('error', () => resolve(null));
    child.on('exit', (code) => {
      if (code === 0) {
        const m = out.trim().match(/(\d+\.\d+\.\d+)/);
        resolve(m ? m[1] : null);
      } else {
        resolve(null);
      }
    });
  });
}

async function sha256File(filePath: string): Promise<string> {
  return new Promise((resolve, reject) => {
    const hash = crypto.createHash('sha256');
    const rs = fs.createReadStream(filePath);
    rs.on('error', reject);
    rs.on('data', (chunk: Buffer) => hash.update(chunk));
    rs.on('end', () => resolve(hash.digest('hex')));
  });
}

async function extractTarGz(archive: string, targetDir: string): Promise<void> {
  return new Promise((resolve, reject) => {
    const child = spawn('tar', ['xzf', archive, '-C', targetDir], { stdio: 'ignore' });
    child.on('error', reject);
    child.on('exit', (code) => code === 0 ? resolve() : reject(new Error(`tar exited with ${code}`)));
  });
}

async function extractZip(archive: string, targetDir: string): Promise<void> {
  if (process.platform === 'win32') {
    return new Promise((resolve, reject) => {
      const child = spawn('powershell', [
        '-Command', `Expand-Archive -Path '${archive}' -DestinationPath '${targetDir}' -Force`
      ], { stdio: 'ignore' });
      child.on('error', reject);
      child.on('exit', (code) => code === 0 ? resolve() : reject(new Error(`unzip exited with ${code}`)));
    });
  } else {
    return new Promise((resolve, reject) => {
      const child = spawn('unzip', ['-o', archive, '-d', targetDir], { stdio: 'ignore' });
      child.on('error', reject);
      child.on('exit', (code) => code === 0 ? resolve() : reject(new Error(`unzip exited with ${code}`)));
    });
  }
}

type StateCallback = (state: InstallState) => void;

async function doInstall(options: { version?: string; force?: boolean }, onState?: StateCallback): Promise<InstallState> {
  const { target, ext } = resolveTarget();
  const installPath = getInstallPath();

  if (options.version) process.env.RAISINDB_VERSION = options.version;

  onState?.({ phase: 'resolving' });

  const cur = await currentVersion();
  const tag = await getLatestReleaseTag();
  const requested = tag.match(/v?(\d+\.\d+\.\d+)/)?.[1] ?? null;

  if (!options.force && cur && requested && cur === requested) {
    const state: InstallState = { phase: 'already-installed', version: cur, installPath };
    onState?.(state);
    return state;
  }

  onState?.({ phase: 'downloading', version: tag, target, downloadedBytes: 0, totalBytes: 0 });

  const artifactName = `raisindb-${tag}-${target}`;
  const assetFile = `${artifactName}.${ext}`;
  const downloadUrl = `https://github.com/${REPO}/releases/download/${tag}/${assetFile}`;
  const sumsUrl = `https://github.com/${REPO}/releases/download/${tag}/SHA256SUMS`;

  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'raisindb-'));
  const archivePath = path.join(tmpDir, assetFile);

  try {
    // Download with progress
    const res = await withRetries(() => fetchWithTimeout(downloadUrl, { headers: makeFetchHeaders() }, 120000));
    if (!res.ok) throw new Error(`Download failed: ${res.status} ${res.statusText}`);
    if (!res.body) throw new Error('No response body');

    const totalBytes = Number(res.headers.get('content-length')) || 0;
    let downloadedBytes = 0;

    const fileStream = createWriteStream(archivePath);
    const progressStream = new Writable({
      write(chunk: Buffer, _encoding, callback) {
        downloadedBytes += chunk.length;
        onState?.({ phase: 'downloading', version: tag, target, downloadedBytes, totalBytes });
        fileStream.write(chunk, callback);
      },
      final(callback) {
        fileStream.end(callback);
      }
    });
    // @ts-ignore
    await pipeline(res.body as any, progressStream);

    // Verify checksum
    onState?.({ phase: 'verifying', version: tag, target });
    try {
      const sumsRes = await fetchWithTimeout(sumsUrl, { headers: makeFetchHeaders() }, 15000);
      if (sumsRes.ok) {
        const sumsText = await sumsRes.text();
        const entry = sumsText.split(/\r?\n/).filter(Boolean)
          .map(l => l.trim().split(/\s+/))
          .find(([, fname]) => fname === assetFile);
        if (entry) {
          const hash = await sha256File(archivePath);
          if (hash !== entry[0]) {
            throw new Error(`Checksum mismatch: expected ${entry[0]}, got ${hash}`);
          }
        }
      }
    } catch (e) {
      if (e instanceof Error && e.message.includes('Checksum mismatch')) throw e;
    }

    // Extract
    onState?.({ phase: 'extracting', version: tag, target });
    if (ext === 'tar.gz') {
      await extractTarGz(archivePath, tmpDir);
    } else {
      await extractZip(archivePath, tmpDir);
    }

    // Find and install binary
    const execName = getExecutableName();
    const innerDir = path.join(tmpDir, artifactName);
    let srcPath = path.join(innerDir, execName);
    if (!fs.existsSync(srcPath)) {
      srcPath = path.join(tmpDir, execName);
      if (!fs.existsSync(srcPath)) {
        throw new Error(`Binary not found in archive. Expected: ${execName}`);
      }
    }

    try { fs.unlinkSync(installPath); } catch {}
    fs.copyFileSync(srcPath, installPath);
    fs.chmodSync(installPath, 0o755);

    const newVer = await currentVersion();
    const state: InstallState = { phase: 'complete', version: newVer || tag, installPath };
    onState?.(state);
    return state;
  } catch (e) {
    const error = e instanceof Error ? e.message : String(e);
    const state: InstallState = { phase: 'error', error };
    onState?.(state);
    throw e;
  } finally {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
}

// Ink wrapper for install with reactive UI
function InstallApp({ options, onDone }: { options: { version?: string; force?: boolean }; onDone: (state: InstallState) => void }) {
  const [state, setState] = React.useState<InstallState>({ phase: 'resolving' });

  React.useEffect(() => {
    doInstall(options, setState)
      .then(onDone)
      .catch(() => onDone(state));
  }, []);

  return React.createElement(ServerInstallUI, { state });
}

export async function serverInstall(options: { version?: string; force?: boolean }): Promise<void> {
  return new Promise((resolve, reject) => {
    const { unmount } = render(
      React.createElement(InstallApp, {
        options,
        onDone: (state: InstallState) => {
          setTimeout(() => {
            unmount();
            if (state.phase === 'error') {
              reject(new Error(state.error));
            } else {
              resolve();
            }
          }, 500);
        }
      })
    );
  });
}

export async function serverStart(args: string[]): Promise<void> {
  const installPath = getInstallPath();

  if (!fs.existsSync(installPath)) {
    await serverInstall({ force: false });
    console.log('');
  }

  const ver = await currentVersion();
  // Clear line and show start banner
  console.log(`  \x1b[38;5;208m▸ RaisinDB\x1b[0m${ver ? ` v${ver}` : ''} starting...`);
  console.log('');

  const child: ChildProcess = spawn(installPath, args, {
    stdio: 'inherit',
    env: { ...process.env },
  });

  const signals: NodeJS.Signals[] = ['SIGINT', 'SIGTERM'];
  for (const sig of signals) {
    process.on(sig, () => child.kill(sig));
  }

  child.on('exit', (code) => {
    process.exit(code ?? 0);
  });
}

export async function serverVersion(): Promise<void> {
  const cur = await currentVersion();
  if (cur) {
    console.log(`  raisindb v${cur}`);
    console.log(`  ${getInstallPath()}`);
  } else {
    console.log('  RaisinDB server is not installed.');
    console.log('  Run: raisindb server install');
  }
}

export async function serverUpdate(): Promise<void> {
  await serverInstall({ force: true });
}
