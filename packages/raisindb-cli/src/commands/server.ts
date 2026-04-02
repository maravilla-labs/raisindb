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
import { ServerInstallUI, type InstallState } from '../components/ServerInstall.js';
import { ServerReady } from '../components/ServerReady.js';

const BIN_NAME = 'raisindb';
const REPO = process.env.RAISINDB_REPO || 'maravilla-labs/raisindb';
const GH_TOKEN = process.env.RAISINDB_GH_TOKEN || process.env.GITHUB_TOKEN || '';
const HTTP_TIMEOUT_MS = Number(process.env.RAISINDB_HTTP_TIMEOUT_MS || 15000);
const HTTP_RETRIES = Math.max(1, Number(process.env.RAISINDB_HTTP_RETRIES || 3));

// --- Path helpers ---

function getRaisinDir(): string {
  const dir = path.join(os.homedir(), '.raisindb');
  fs.mkdirSync(dir, { recursive: true });
  return dir;
}

function getBinDir(): string {
  const dir = path.join(getRaisinDir(), 'bin');
  fs.mkdirSync(dir, { recursive: true });
  return dir;
}

function getExecutableName(): string {
  return process.platform === 'win32' ? `${BIN_NAME}.exe` : BIN_NAME;
}

function getInstallPath(): string {
  return path.join(getBinDir(), getExecutableName());
}

function getPidPath(): string {
  return path.join(getRaisinDir(), 'server.pid');
}

function getLogPath(): string {
  return path.join(getRaisinDir(), 'server.log');
}

// --- Platform / network helpers ---

function resolveTarget(): { target: string; ext: string } {
  const p = process.platform;
  const a = process.arch;
  if (p === 'linux' && a === 'x64') return { target: 'x86_64-unknown-linux-gnu', ext: 'tar.gz' };
  if (p === 'darwin' && a === 'x64') return { target: 'x86_64-apple-darwin', ext: 'tar.gz' };
  if (p === 'darwin' && a === 'arm64') return { target: 'aarch64-apple-darwin', ext: 'tar.gz' };
  if (p === 'win32' && a === 'x64') return { target: 'x86_64-pc-windows-msvc', ext: 'zip' };
  throw new Error(`Unsupported platform/arch: ${p}/${a}`);
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
    try { return await fn(); } catch (e) { lastErr = e; if (attempt + 1 < tries) await sleep(1000 * Math.pow(2, attempt)); }
  }
  throw lastErr;
}

async function getLatestReleaseTag(): Promise<string> {
  const version = process.env.RAISINDB_VERSION;
  if (version && version !== 'latest') return version;
  const headers = makeFetchHeaders();
  try {
    const res = await withRetries(() => fetchWithTimeout(`https://api.github.com/repos/${REPO}/releases/latest`, { headers }, HTTP_TIMEOUT_MS));
    if (!res.ok) throw new Error(`API ${res.status}`);
    const json = await res.json() as { tag_name?: string };
    if (!json.tag_name) throw new Error('No tag');
    return json.tag_name;
  } catch {
    const res = await fetchWithTimeout(`https://github.com/${REPO}/releases/latest`, { headers, redirect: 'manual' }, 10000);
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
    child.on('exit', (code) => { const m = code === 0 ? out.trim().match(/(\d+\.\d+\.\d+)/) : null; resolve(m ? m[1] : null); });
  });
}

async function sha256File(filePath: string): Promise<string> {
  return new Promise((resolve, reject) => {
    const hash = crypto.createHash('sha256');
    const rs = fs.createReadStream(filePath);
    rs.on('error', reject);
    rs.on('data', (chunk) => hash.update(chunk));
    rs.on('end', () => resolve(hash.digest('hex')));
  });
}

async function extractTarGz(archive: string, targetDir: string): Promise<void> {
  return new Promise((resolve, reject) => {
    const child = spawn('tar', ['xzf', archive, '-C', targetDir], { stdio: 'ignore' });
    child.on('error', reject);
    child.on('exit', (code) => code === 0 ? resolve() : reject(new Error(`tar exited ${code}`)));
  });
}

async function extractZip(archive: string, targetDir: string): Promise<void> {
  if (process.platform === 'win32') {
    return new Promise((resolve, reject) => {
      const child = spawn('powershell', ['-Command', `Expand-Archive -Path '${archive}' -DestinationPath '${targetDir}' -Force`], { stdio: 'ignore' });
      child.on('error', reject);
      child.on('exit', (code) => code === 0 ? resolve() : reject(new Error(`unzip exited ${code}`)));
    });
  }
  return new Promise((resolve, reject) => {
    const child = spawn('unzip', ['-o', archive, '-d', targetDir], { stdio: 'ignore' });
    child.on('error', reject);
    child.on('exit', (code) => code === 0 ? resolve() : reject(new Error(`unzip exited ${code}`)));
  });
}

// --- PID management ---

function readPid(): number | null {
  try {
    const pid = parseInt(fs.readFileSync(getPidPath(), 'utf-8').trim(), 10);
    try { process.kill(pid, 0); return pid; } catch { fs.unlinkSync(getPidPath()); return null; }
  } catch { return null; }
}

function writePid(pid: number): void { fs.writeFileSync(getPidPath(), String(pid)); }
function removePid(): void { try { fs.unlinkSync(getPidPath()); } catch {} }

// --- Install (Ink TUI) ---

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
      final(callback) { fileStream.end(callback); }
    });
    // @ts-ignore
    await pipeline(res.body as any, progressStream);

    onState?.({ phase: 'verifying', version: tag, target });
    try {
      const sumsRes = await fetchWithTimeout(sumsUrl, { headers: makeFetchHeaders() }, 15000);
      if (sumsRes.ok) {
        const sumsText = await sumsRes.text();
        const entry = sumsText.split(/\r?\n/).filter(Boolean).map(l => l.trim().split(/\s+/)).find(([, f]) => f === assetFile);
        if (entry) { const hash = await sha256File(archivePath); if (hash !== entry[0]) throw new Error('Checksum mismatch'); }
      }
    } catch (e) { if (e instanceof Error && e.message.includes('Checksum')) throw e; }

    onState?.({ phase: 'extracting', version: tag, target });
    if (ext === 'tar.gz') await extractTarGz(archivePath, tmpDir);
    else await extractZip(archivePath, tmpDir);

    const execName = getExecutableName();
    let srcPath = path.join(tmpDir, artifactName, execName);
    if (!fs.existsSync(srcPath)) srcPath = path.join(tmpDir, execName);
    if (!fs.existsSync(srcPath)) throw new Error(`Binary not found: ${execName}`);

    try { fs.unlinkSync(installPath); } catch {}
    fs.copyFileSync(srcPath, installPath);
    fs.chmodSync(installPath, 0o755);

    const newVer = await currentVersion();
    const state: InstallState = { phase: 'complete', version: newVer || tag, installPath };
    onState?.(state);
    return state;
  } catch (e) {
    const error = e instanceof Error ? e.message : String(e);
    onState?.({ phase: 'error', error });
    throw e;
  } finally {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
}

function InstallApp({ options, onDone }: { options: { version?: string; force?: boolean }; onDone: (state: InstallState) => void }) {
  const [state, setState] = React.useState<InstallState>({ phase: 'resolving' });
  React.useEffect(() => { doInstall(options, setState).then(onDone).catch(() => onDone(state)); }, []);
  return React.createElement(ServerInstallUI, { state });
}

export async function serverInstall(options: { version?: string; force?: boolean }): Promise<void> {
  return new Promise((resolve, reject) => {
    const { unmount } = render(React.createElement(InstallApp, {
      options,
      onDone: (state: InstallState) => {
        setTimeout(() => { unmount(); state.phase === 'error' ? reject(new Error(state.error)) : resolve(); }, 500);
      }
    }));
  });
}

// --- ANSI helpers ---
const dim = (s: string) => `\x1b[2m${s}\x1b[0m`;
const bold = (s: string) => `\x1b[1m${s}\x1b[0m`;
const green = (s: string) => `\x1b[32m${s}\x1b[0m`;
const yellow = (s: string) => `\x1b[33m${s}\x1b[0m`;
const cyan = (s: string) => `\x1b[36m${s}\x1b[0m`;
const red = (s: string) => `\x1b[31m${s}\x1b[0m`;
const orange = (s: string) => `\x1b[38;5;208m${s}\x1b[0m`;

// --- Server Start ---

export async function serverStart(args: string[], options: { verbose?: boolean; production?: boolean; detach?: boolean; port?: string; pgwirePort?: string }): Promise<void> {
  const installPath = getInstallPath();

  // Auto-install if needed
  if (!fs.existsSync(installPath)) {
    await serverInstall({ force: false });
    console.log('');
  }

  // Check if already running
  const existingPid = readPid();
  if (existingPid) {
    console.log(`  RaisinDB is already running (PID ${existingPid})`);
    console.log(`  Run: ${dim('raisindb server stop')}`);
    return;
  }

  const ver = await currentVersion();
  const devMode = !options.production;
  const httpPort = options.port || '8080';
  const pgwirePort = options.pgwirePort || '5432';

  // Build server args
  const serverArgs = [...args];
  if (devMode && !serverArgs.includes('--dev-mode')) serverArgs.push('--dev-mode');
  if (options.port) serverArgs.push('--http-port', options.port);

  // First run detection — generate and show password only on fresh start
  const dataDir = path.resolve('.data', 'rocksdb');
  const isFirstRun = !fs.existsSync(dataDir);
  let generatedPassword: string | null = null;

  if (isFirstRun && devMode && !serverArgs.includes('--initial-admin-password') && !process.env.RAISIN_ADMIN_PASSWORD) {
    // Generate a random password for first run
    generatedPassword = crypto.randomBytes(12).toString('base64url').slice(0, 16);
    serverArgs.push('--initial-admin-password', generatedPassword);
  }

  // Set log level — quiet by default, verbose shows everything
  const rustLog = options.verbose
    ? 'info'
    : 'warn,raisin_server=warn';

  const logFile = getLogPath();
  const logStream = createWriteStream(logFile, { flags: 'a' });

  // Start server process
  const child: ChildProcess = spawn(installPath, serverArgs, {
    stdio: ['ignore', 'pipe', 'pipe'],
    env: { ...process.env, RUST_LOG: rustLog },
    detached: options.detach || false,
  });

  if (!child.pid) {
    console.error(red('  Failed to start server process'));
    process.exit(1);
  }

  writePid(child.pid);

  // Capture admin password from server output
  let adminPassword: string | null = null;
  let serverReady = false;
  let startupError: string | null = null;

  function handleLine(line: string) {
    // Always write to log file
    logStream.write(line + '\n');

    // Parse admin password from SUPERADMIN box output
    // Format: ║ Password:   abc123def                                              ║
    const pwMatch = line.match(/Password:\s+(\S+)/);
    if (pwMatch && !adminPassword) {
      adminPassword = pwMatch[1].replace(/║/g, '').trim();
    }

    // Detect fatal errors
    if (line.includes('JWT_SECRET is not set') || line.includes('exit code: 1')) {
      startupError = line.replace(/.*?(JWT_SECRET|exit)/, '$1').trim();
    }

    // Show in verbose mode
    if (options.verbose) {
      process.stderr.write(line + '\n');
    }
  }

  // Pipe stdout/stderr through line parser
  let buffer = '';
  const processData = (data: Buffer) => {
    buffer += data.toString();
    const lines = buffer.split('\n');
    buffer = lines.pop() || '';
    for (const line of lines) {
      if (line.trim()) handleLine(line);
    }
  };

  child.stdout?.on('data', processData);
  child.stderr?.on('data', processData);

  // Detach the server process so Node can exit
  child.unref();
  // Disconnect pipes after capturing initial output
  const disconnectPipes = () => {
    child.stdout?.removeAllListeners();
    child.stderr?.removeAllListeners();
    child.stdout?.destroy();
    child.stderr?.destroy();
  };

  // Wait for ready (health check)
  console.log(`  ${dim('Starting RaisinDB...')}`);

  for (let i = 0; i < 30; i++) {
    await sleep(500);
    if (startupError) {
      process.stdout.write('\x1b[1A\x1b[2K');
      console.log(red(`  Server failed to start: ${startupError}`));
      console.log(dim(`  Check logs: raisindb server logs`));
      disconnectPipes();
      removePid();
      logStream.end();
      process.exit(1);
    }
    try {
      const res = await fetchWithTimeout(`http://localhost:${httpPort}/health`, {}, 1000);
      if (res.ok) { serverReady = true; break; }
    } catch {}
  }

  // Stop capturing output — let server run independently
  disconnectPipes();
  logStream.end();

  // Clear "Starting..." line
  process.stdout.write('\x1b[1A\x1b[2K');

  // Render Ink banner
  const { unmount: unmountBanner } = render(
    React.createElement(ServerReady, {
      version: ver || undefined,
      devMode,
      httpPort,
      pgwirePort,
      adminPassword: isFirstRun ? generatedPassword : null,
      dataDir,
      isFirstRun,
      pid: child.pid!,
    })
  );

  // Wait for animation to complete, then unmount and exit
  await sleep(isFirstRun ? 1500 : 500);
  unmountBanner();
}

function printBanner(ver: string | null, devMode: boolean, httpPort: string, pgwirePort: string, adminPassword: string | null, dataDir?: string, isFirstRun?: boolean) {
  console.log('');
  console.log(`  ${orange('RaisinDB')}${ver ? dim(` v${ver}`) : ''}${devMode ? `  ${yellow('Development Mode')}` : `  ${green('Production')}`}`);
  console.log('');
  console.log(`  ${dim('HTTP API')}     ${cyan(`http://localhost:${httpPort}`)}`);
  console.log(`  ${dim('PostgreSQL')}   ${cyan(`postgresql://localhost:${pgwirePort}`)}`);
  console.log(`  ${dim('Admin UI')}     ${cyan(`http://localhost:${httpPort}/admin`)}`);
  if (dataDir) {
    console.log(`  ${dim('Data')}         ${dim(dataDir)}`);
  }

  if (adminPassword) {
    console.log('');
    console.log(`  ${dim('Username')}     ${bold('admin')}`);
    console.log(`  ${dim('Password')}     ${bold(adminPassword)}`);
    console.log('');
    console.log(`  ${yellow('!')} Save this password — it won't be shown again.`);
  }

  console.log('');
  if (devMode) {
    console.log(`  ${yellow('!')} ${dim('Dev mode — insecure defaults. Use')} ${bold('--production')} ${dim('for secure config.')}`);
  }

  if (isFirstRun) {
    console.log('');
    console.log(`  ${dim('Get started:')}`);
    console.log(`    ${dim('$')} psql -h localhost -p ${pgwirePort} -U admin`);
    console.log(`    ${dim('$')} open ${cyan(`http://localhost:${httpPort}/admin`)}`);
    console.log(`    ${dim('$')} raisindb shell`);
  }
  console.log('');
}

// --- Server Stop ---

export async function serverStop(quiet = false): Promise<void> {
  const pid = readPid();
  if (!pid) {
    if (!quiet) console.log('  RaisinDB is not running');
    return;
  }
  try {
    process.kill(pid, 'SIGTERM');
    removePid();
    console.log(`  ${green('✓')} RaisinDB stopped (PID ${pid})`);
  } catch {
    removePid();
    console.log(`  Process ${pid} not found, cleaned up PID file`);
  }
}

// --- Server Status ---

export async function serverStatus(): Promise<void> {
  const pid = readPid();
  const ver = await currentVersion();

  if (pid) {
    console.log(`  ${green('✓')} RaisinDB running (PID ${pid})`);
    if (ver) console.log(`    Version:   v${ver}`);
    console.log(`    Logs:      ${getLogPath()}`);
    try {
      const res = await fetchWithTimeout('http://localhost:8080/health', {}, 3000);
      console.log(`    HTTP:      ${res.ok ? green('healthy') : yellow(String(res.status))}`);
    } catch {
      console.log(`    HTTP:      ${red('unreachable')}`);
    }
  } else {
    console.log('  RaisinDB is not running');
    if (ver) console.log(`    Installed: v${ver}`);
    else console.log(`    Run: ${dim('raisindb server install')}`);
  }
}

// --- Server Logs ---

export async function serverLogs(options: { follow?: boolean; lines?: string }): Promise<void> {
  const logFile = getLogPath();
  if (!fs.existsSync(logFile)) {
    console.log('  No log file found. Start the server first.');
    return;
  }

  const numLines = parseInt(options.lines || '50', 10);

  if (options.follow) {
    const tail = spawn('tail', ['-f', '-n', String(numLines), logFile], { stdio: 'inherit' });
    process.on('SIGINT', () => { tail.kill(); process.exit(0); });
    tail.on('exit', () => process.exit(0));
  } else {
    const content = fs.readFileSync(logFile, 'utf-8');
    const lines = content.trim().split('\n');
    console.log(lines.slice(-numLines).join('\n'));
  }
}

// --- Server Version ---

export async function serverVersion(): Promise<void> {
  const cur = await currentVersion();
  if (cur) {
    console.log(`  raisindb v${cur}`);
    console.log(`  ${getInstallPath()}`);
  } else {
    console.log('  RaisinDB server is not installed.');
    console.log(`  Run: ${dim('raisindb server install')}`);
  }
}

// --- Server Update ---

export async function serverUpdate(): Promise<void> {
  await serverInstall({ force: true });
}
