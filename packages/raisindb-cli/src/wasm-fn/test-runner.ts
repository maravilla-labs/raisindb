/**
 * `raisindb function test` — the two test lanes of a wasm function.
 *
 * NATIVE (the default) runs the project's own suite with its host toolchain:
 * the SDKs are written so a handler is an ordinary function against a mock
 * host, which is the fast lane and needs no server at all.
 *
 * SERVER (`--server`) replays `tests/server.json` through `function run`, so
 * the assertions are made against the real runtime, the real host gateway and
 * the real `entry_file` routing. Each case may name a `handler`, which is how
 * one artifact's several handlers are covered from one file.
 */

import fs from 'fs';
import path from 'path';
import { runCommand } from './toolchains.js';
import { executeRun, type ServerOptions } from './run.js';
import { resolveRunTarget } from './run-target.js';
import type { WasmLang, WasmProject } from './types.js';

/** One case in `tests/server.json`. */
export interface ServerCase {
  /** Handler to call; defaults to the Function node's own. */
  handler?: string;
  /** JSON input for the handler. */
  input?: unknown;
  /** Expected output — an object is matched as a SUBSET (see `matchesExpectation`). */
  expect?: unknown;
}

/** The native test command per language, run in the project directory. */
export function defaultTestCommand(lang: WasmLang): string {
  switch (lang) {
    case 'assemblyscript':
      // `npm test` builds the core module and runs `node --test`, which drives
      // the guest through the SDK's mock host — the same no-server guarantee
      // `cargo test` and `go test` give.
      return 'npm test';
    case 'rust':
      return 'cargo test';
    case 'go':
      return 'go test ./...';
    case 'ts':
      // The scaffold's package.json maps `test` to `vitest run`, so this works
      // from a local devDependency and needs no global vitest.
      return 'npm test';
  }
}

/** Run the project's native suite; returns its exit code. */
export async function runNativeTests(project: WasmProject): Promise<number> {
  const command = defaultTestCommand(project.spec.lang);
  console.log(`> ${command}  (${project.dir})`);
  const result = await runCommand(command, project.dir);
  return result.code;
}

/** Absolute path of a project's server-case file. */
export function serverCasesFile(projectDir: string): string {
  return path.join(projectDir, 'tests', 'server.json');
}

/**
 * Where a SOURCE-shipping function keeps its cases.
 *
 * A `js` or `starlark` function has no build project, so its cases live beside
 * its node — and must be HIDDEN, because everything else under `content/` is
 * uploaded: `sync/mapping.ts` skips dotfiles, and so does the package
 * collector. A `tests/server.json` there would become a node.
 */
export function sourceCasesFile(nodeDir: string): string {
  return path.join(nodeDir, '.tests.json');
}

/** Load cases from an explicit file. Returns `[]` when it does not exist. */
export function loadCasesFrom(file: string): ServerCase[] {
  if (!fs.existsSync(file)) return [];
  let parsed: unknown;
  try {
    parsed = JSON.parse(fs.readFileSync(file, 'utf-8'));
  } catch (error) {
    throw new Error(
      `${file}: not valid JSON — ${error instanceof Error ? error.message : String(error)}`
    );
  }
  if (!Array.isArray(parsed)) throw new Error(`${file}: expected a JSON array of cases`);
  return parsed as ServerCase[];
}

/** Load `tests/server.json`. Returns `[]` when the project has none. */
export function loadServerCases(projectDir: string): ServerCase[] {
  const file = serverCasesFile(projectDir);
  if (!fs.existsSync(file)) return [];
  let parsed: unknown;
  try {
    parsed = JSON.parse(fs.readFileSync(file, 'utf-8'));
  } catch (error) {
    throw new Error(
      `${file}: not valid JSON — ${error instanceof Error ? error.message : String(error)}`
    );
  }
  if (!Array.isArray(parsed)) {
    throw new Error(`${file}: expected an array of { handler?, input, expect } cases`);
  }
  return parsed as ServerCase[];
}

/**
 * Does the actual output satisfy the expectation?
 *
 * Objects match as a SUBSET — a case asserts the fields it cares about and a
 * handler may return more (an echoed `abi`, a timestamp) without breaking every
 * test. Arrays and primitives match exactly, because a "subset array" has no
 * meaning a test author would predict.
 */
export function matchesExpectation(actual: unknown, expected: unknown): boolean {
  if (expected === undefined) return true;
  if (expected === null || typeof expected !== 'object') return actual === expected;
  if (Array.isArray(expected)) {
    if (!Array.isArray(actual) || actual.length !== expected.length) return false;
    return expected.every((value, index) => matchesExpectation(actual[index], value));
  }
  if (!actual || typeof actual !== 'object' || Array.isArray(actual)) return false;
  const actualObj = actual as Record<string, unknown>;
  return Object.entries(expected as Record<string, unknown>).every(([key, value]) =>
    matchesExpectation(actualObj[key], value)
  );
}

/** What one server case did. */
export interface ServerCaseResult {
  index: number;
  handler?: string;
  passed: boolean;
  /** Why it failed — an execution error, or the diff line. */
  detail?: string;
  durationMs?: number;
}

/** Options for the server lane. */
export interface ServerTestOptions extends ServerOptions {
  /** Per-case timeout in milliseconds. */
  timeoutMs?: number;
  /** Injected for tests. */
  fetchImpl?: typeof fetch;
}

/**
 * Replay every case in `tests/server.json` through the dev-loop run path.
 *
 * The target is resolved per case, because a case's `handler` selects which
 * Function node (and therefore which `entry_file`) the run goes through.
 */
export async function runServerCases(
  project: WasmProject,
  cases: ServerCase[],
  options: ServerTestOptions = {}
): Promise<ServerCaseResult[]> {
  const results: ServerCaseResult[] = [];
  for (const [index, testCase] of cases.entries()) {
    const label = testCase.handler ? `case ${index + 1} (${testCase.handler})` : `case ${index + 1}`;
    try {
      const target = resolveRunTarget(project.dir, { handler: testCase.handler });
      const { outcome } = await executeRun(
        target,
        {
          input: testCase.input ?? {},
          timeoutMs: options.timeoutMs,
          server: options.server,
          repo: options.repo,
          branch: options.branch,
          fetchImpl: options.fetchImpl,
        }
      );
      if (!outcome.success) {
        results.push({
          index,
          handler: testCase.handler,
          passed: false,
          detail: outcome.error || 'the function reported failure with no message',
          durationMs: outcome.durationMs,
        });
        continue;
      }
      const passed = matchesExpectation(outcome.result, testCase.expect);
      results.push({
        index,
        handler: testCase.handler,
        passed,
        detail: passed
          ? undefined
          : `expected ${JSON.stringify(testCase.expect)}, got ${JSON.stringify(outcome.result)}`,
        durationMs: outcome.durationMs,
      });
    } catch (error) {
      results.push({
        index,
        handler: testCase.handler,
        passed: false,
        detail: `${label}: ${error instanceof Error ? error.message : String(error)}`,
      });
    }
  }
  return results;
}
