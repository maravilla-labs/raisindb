#!/usr/bin/env node
/**
 * WebAssembly functions demo — end-to-end smoke test.
 *
 * Proves the whole developer loop against a running server:
 *   authenticate -> `raisindb deploy . --install` -> invoke every Function
 *   node -> assert the three languages answer IDENTICALLY, and that
 *   `greet-rust` and `greet-rust-shout` — two Function nodes, ONE uploaded
 *   artifact — each run their own handler.
 *
 * The artifacts must be built first (they are build output, not source):
 *
 *   make wasm-sdks-build          # all three, needs cargo / tinygo / node
 *
 * A missing artifact is a hard failure rather than a skip: `raisindb deploy`
 * validates that every `language: wasm` node's `entry_file` target exists, so
 * the deploy this script runs would fail anyway — better to say why up front.
 * Pass `--skip-deploy` to invoke against an already-installed package (that is
 * the only mode in which a partially-built package is meaningful).
 *
 * Run: node smoke.mjs [--server URL] [--repo NAME] [--skip-deploy]
 */

import fs from 'node:fs';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(HERE, '..', '..');
const CLI = path.join(REPO_ROOT, 'packages', 'raisindb-cli', 'dist', 'index.js');

const argv = process.argv.slice(2);
const flag = (name) => argv.includes(name);
const value = (name, fallback) => {
  const i = argv.indexOf(name);
  return i >= 0 && argv[i + 1] ? argv[i + 1] : fallback;
};

const SERVER = value('--server', process.env.RAISINDB_SERVER ?? 'http://localhost:8080');
const REPO = value('--repo', process.env.RAISINDB_REPO ?? 'wasmdemo');
const TENANT = value('--tenant', process.env.RAISIN_TENANT ?? 'default');
const USER = process.env.RAISIN_USER ?? 'admin';
const PASSWORD = process.env.RAISIN_PASSWORD ?? 'Admin12345!@#';
const SKIP_DEPLOY = flag('--skip-deploy');
const GREET = 'Ada';

/** The Function nodes this package installs, and what each must answer. */
const FUNCTIONS = [
  {
    name: 'greet-rust',
    language: 'rust',
    node: 'content/functions/lib/demo/greet-rust',
    // `entry_file: main.wasm` -> handler "default".
    artifact: 'content/functions/lib/demo/greet-rust/main.wasm',
    greeting: `Hello, ${GREET}!`,
  },
  {
    name: 'greet-rust-shout',
    language: 'rust',
    node: 'content/functions/lib/demo/greet-rust-shout',
    // `entry_file: ../greet-rust/main.wasm:shout` — the SAME bytes as
    // greet-rust, a different handler. Nothing is uploaded for this node.
    artifact: 'content/functions/lib/demo/greet-rust/main.wasm',
    greeting: `HELLO, ${GREET.toUpperCase()}!`,
  },
  {
    name: 'greet-go',
    language: 'go',
    node: 'content/functions/lib/demo/greet-go',
    artifact: 'content/functions/lib/demo/greet-go/main.wasm',
    greeting: `Hello, ${GREET}!`,
  },
  {
    name: 'greet-ts',
    language: 'ts',
    node: 'content/functions/lib/demo/greet-ts',
    artifact: 'content/functions/lib/demo/greet-ts/main.wasm',
    greeting: `Hello, ${GREET}!`,
  },
];

/** How to build each language's artifact, quoted in the missing-artifact error. */
const BUILD_HINT = {
  rust: 'make wasm-sdks-build LANGS=rust   (or: cd wasm/demo/greet-rust && cargo build --release --target wasm32-wasip2)',
  go: 'make wasm-sdks-build LANGS=go     (needs tinygo)',
  ts: 'make wasm-sdks-build LANGS=ts     (needs node + jco)',
};

function assert(cond, message) {
  if (!cond) throw new Error(`Assertion failed: ${message}`);
}

async function api(pathname, { method = 'GET', token, body } = {}) {
  const response = await fetch(`${SERVER}${pathname}`, {
    method,
    headers: {
      'content-type': 'application/json',
      ...(token ? { authorization: `Bearer ${token}` } : {}),
    },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await response.text();
  let json;
  try {
    json = text ? JSON.parse(text) : null;
  } catch {
    json = null;
  }
  return { ok: response.ok, status: response.status, json, text };
}

async function login() {
  const res = await api(`/api/raisindb/sys/${TENANT}/auth`, {
    method: 'POST',
    body: { username: USER, password: PASSWORD, interface: 'console' },
  });
  assert(res.ok && res.json?.token, `login failed (${res.status}): ${res.text.slice(0, 300)}`);
  return res.json.token;
}

/** Create the repository if it is not there yet; an existing one is fine. */
async function ensureRepo(token) {
  const res = await api('/api/repositories', {
    method: 'POST',
    token,
    body: { repo_id: REPO, description: 'WebAssembly functions example' },
  });
  if (res.ok) {
    console.log(`✅ Repository '${REPO}' created`);
  } else {
    console.log(`ℹ️  Repository '${REPO}' not created (${res.status}) — assuming it exists`);
  }
}

/** Run the real CLI, so this test exercises the command users actually type. */
function deploy(token) {
  assert(
    fs.existsSync(CLI),
    `CLI not built: ${CLI}\n   Build it with: pnpm --filter @raisindb/cli run build`,
  );
  console.log(`\n$ raisindb deploy . --install --repo ${REPO}\n`);
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, [CLI, 'deploy', '.', '--install', '--repo', REPO], {
      cwd: HERE,
      stdio: 'inherit',
      env: {
        ...process.env,
        RAISINDB_SERVER: SERVER,
        RAISINDB_TOKEN: token,
        RAISINDB_REPO: REPO,
      },
    });
    child.on('error', reject);
    child.on('exit', (code) =>
      code === 0 ? resolve() : reject(new Error(`raisindb deploy exited with ${code}`)),
    );
  });
}

/** Invoke one function synchronously and return its result object. */
async function invoke(token, name) {
  const res = await api(`/api/functions/${REPO}/${name}/invoke`, {
    method: 'POST',
    token,
    body: { input: { name: GREET }, sync: true },
  });
  assert(res.ok, `invoke ${name} failed (${res.status}): ${res.text.slice(0, 400)}`);
  assert(!res.json?.error, `invoke ${name} returned an error: ${res.json?.error}`);
  const result = res.json?.result;
  assert(result && typeof result === 'object', `invoke ${name} returned no result object`);
  return { result, logs: res.json?.logs ?? [], durationMs: res.json?.duration_ms };
}

async function main() {
  console.log(`WebAssembly functions smoke test against ${SERVER} (repo '${REPO}')\n`);

  // 1. Artifacts. Checked before anything touches the network: a missing one
  //    fails `deploy`'s own wasm validation, and the useful message is here.
  const missing = FUNCTIONS.filter((fn) => !fs.existsSync(path.join(HERE, fn.artifact)));
  const unique = [...new Set(missing.map((fn) => fn.language))];
  if (missing.length > 0 && !SKIP_DEPLOY) {
    throw new Error(
      `artifact(s) not built:\n` +
        missing.map((fn) => `   ${fn.name} -> ${fn.artifact}`).join('\n') +
        `\n\n   Build them:\n` +
        unique.map((lang) => `   ${BUILD_HINT[lang]}`).join('\n'),
    );
  }
  const selected = FUNCTIONS.filter((fn) => !missing.includes(fn));
  assert(selected.length > 0, 'no artifact is built — nothing to invoke');
  for (const fn of selected) {
    const bytes = fs.statSync(path.join(HERE, fn.artifact)).size;
    console.log(`   ${fn.name.padEnd(18)} ${fn.artifact} (${(bytes / 1024).toFixed(0)} KB)`);
  }

  // 2. Authenticate, 3. deploy through the CLI.
  const token = await login();
  console.log(`\n✅ Authenticated as ${USER}`);
  if (SKIP_DEPLOY) {
    console.log('⏭️  --skip-deploy: invoking the already-installed package');
  } else {
    await ensureRepo(token);
    await deploy(token);
    console.log('✅ Package deployed and installed');
  }

  // 4. Invoke every function that has an artifact.
  const answers = new Map();
  for (const fn of selected) {
    const { result, logs, durationMs } = await invoke(token, fn.name);
    console.log(
      `\n▶ ${fn.name} (${fn.language})  ${durationMs ?? '?'} ms\n` +
        `  result: ${JSON.stringify(result)}\n` +
        `  logs:   ${logs.length ? logs.join(' | ') : '(none)'}`,
    );
    assert(
      result.greeting === fn.greeting,
      `${fn.name} greeting: expected ${JSON.stringify(fn.greeting)}, got ${JSON.stringify(result.greeting)}`,
    );
    assert(
      typeof result.people === 'number',
      `${fn.name} must report a numeric people count (the host gateway call), got ${JSON.stringify(result.people)}`,
    );
    assert(logs.length > 0, `${fn.name} must produce at least one log line`);
    answers.set(fn.name, result);
  }

  // 5. The three languages must agree — same greeting, same host answer.
  const defaults = selected.filter((fn) => fn.name !== 'greet-rust-shout');
  const greetings = new Set(defaults.map((fn) => answers.get(fn.name).greeting));
  const people = new Set(selected.map((fn) => answers.get(fn.name).people));
  assert(
    greetings.size === 1,
    `languages disagree on the greeting: ${JSON.stringify([...greetings])}`,
  );
  assert(people.size === 1, `languages disagree on the people count: ${JSON.stringify([...people])}`);
  console.log(
    `\n✅ ${defaults.length} language(s) agree: ${[...greetings][0]} / people=${[...people][0]}`,
  );

  // 6. One artifact, two handlers.
  if (answers.has('greet-rust') && answers.has('greet-rust-shout')) {
    const plain = answers.get('greet-rust');
    const shout = answers.get('greet-rust-shout');
    assert(shout.greeting === plain.greeting.toUpperCase(), 'shout must be the greeting in capitals');
    assert(plain.handler === 'default' && shout.handler === 'shout', 'each node must run its own handler');
    console.log('✅ One uploaded main.wasm served both greet-rust (default) and greet-rust-shout (shout)');
  }

  console.log('\n🎉 Smoke test PASSED');
  process.exit(0);
}

main().catch((err) => {
  console.error(`\n❌ Smoke test FAILED: ${err.message || err}`);
  process.exit(1);
});
