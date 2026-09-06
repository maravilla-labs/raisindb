/**
 * `raisindb function run` and `raisindb function test` — the halves of the wasm
 * dev loop that need a server (or, for the native test lane, a toolchain).
 *
 * Kept apart from `function.ts` (build/doctor, both strictly offline) so it is
 * obvious which commands dial out.
 *
 * Exit codes follow `flow doctor`: 0 success, 1 the function or a case failed,
 * 2 nothing to run / bad usage.
 */

import path from 'path';
import React from 'react';
import { render } from 'ink';
import { FunctionRun } from '../components/FunctionRun.js';
import { discoverFunctionNodes, discoverProjects, findPackageRoot } from '../wasm-fn/discover.js';
import { executeRun, type ServerOptions } from '../wasm-fn/run.js';
import { resolveInput, resolveRunTarget } from '../wasm-fn/run-target.js';
import {
  loadCasesFrom,
  sourceCasesFile,
  type ServerCase,
  loadServerCases,
  runNativeTests,
  runServerCases,
  serverCasesFile,
} from '../wasm-fn/test-runner.js';
import type { WasmProject } from '../wasm-fn/types.js';

/** Options for `raisindb function run`. */
export interface FunctionRunOptions extends ServerOptions {
  /** Inline JSON input. */
  input?: string;
  /** Path to a JSON file holding the input. */
  inputFile?: string;
  /** Call this handler instead of the node's `entry_file` one. */
  handler?: string;
  /** Timeout in milliseconds. */
  timeout?: string;
  /** Print one JSON object instead of the live view. */
  json?: boolean;
}

/** Options for `raisindb function test`. */
export interface FunctionTestOptions {
  /**
   * `--server` switches to the server lane; `--server <url>` also says which
   * server. Boolean and string share one flag because the lane and the target
   * are the same decision — there is no "server lane, but nowhere".
   */
  server?: boolean | string;
  repo?: string;
  branch?: string;
  /** Per-case timeout in milliseconds. */
  timeout?: string;
}

/** Parse a millisecond option, rejecting the values that silently become NaN. */
function parseTimeout(value: string | undefined): number | undefined {
  if (value === undefined) return undefined;
  const ms = Number(value);
  if (!Number.isFinite(ms) || ms <= 0) throw new Error(`--timeout must be a positive number of milliseconds (got ${value})`);
  return ms;
}

/** Run one function; returns the process exit code. */
export async function functionRun(
  target: string | undefined,
  options: FunctionRunOptions = {}
): Promise<number> {
  const resolved = resolveRunTarget(target, { handler: options.handler });
  const input = resolveInput(options.input, options.inputFile);
  const timeoutMs = parseTimeout(options.timeout);
  const runOptions = {
    input,
    timeoutMs,
    server: options.server,
    repo: options.repo,
    branch: options.branch,
  };

  if (options.json) {
    const { outcome, plan } = await executeRun(resolved, runOptions);
    console.log(
      JSON.stringify(
        {
          function: resolved.node.name,
          handler: resolved.handler,
          mode: plan.mode,
          reason: plan.reason,
          success: outcome.success,
          result: outcome.result,
          error: outcome.error,
          duration_ms: outcome.durationMs,
          logs: outcome.logs,
        },
        null,
        2
      )
    );
    return outcome.success ? 0 : 1;
  }

  const title = `${resolved.node.name}:${resolved.handler}`;
  return new Promise<number>((resolve) => {
    const { unmount } = render(
      React.createElement(FunctionRun, {
        title,
        execute: (emit) => executeRun(resolved, runOptions, emit),
        onDone: (code: number) => {
          unmount();
          resolve(code);
        },
      })
    );
  });
}

/** The single project a test target names, or an explanatory throw. */
function soleProject(target: string | undefined): WasmProject {
  const start = path.resolve(target || process.cwd());
  const { projects, failures } = discoverProjects(start);
  for (const failure of failures) console.error(`x ${failure.file}: ${failure.error}`);
  if (projects.length === 0) {
    throw new Error(
      `No raisin.build.yaml found under ${start}.\n` +
        'Point `function test` at a wasm project directory.'
    );
  }
  if (projects.length > 1) {
    const root = findPackageRoot(start) || start;
    const list = projects.map((p) => `  ${path.relative(root, p.dir)}`).join('\n');
    throw new Error(`${projects.length} wasm projects under ${start}:\n${list}\nName one.`);
  }
  return projects[0];
}


/**
 * The source-shipping Function node a target names, if it names one.
 *
 * Returns null for a wasm target so the ordinary project path runs. Resolution
 * is deliberately by NODE rather than by project: `js` and `starlark` have no
 * project directory to point at.
 */
function sourceFunctionTarget(
  target: string | undefined
): { dir: string; language: string } | null {
  const start = path.resolve(target || process.cwd());
  const packageRoot = findPackageRoot(start);
  if (!packageRoot) return null;
  const node = discoverFunctionNodes(packageRoot).find(
    (n) => (n.dir === start || n.file === start) && n.language !== 'wasm'
  );
  if (!node) return null;
  return { dir: node.dir, language: node.language };
}

/** Run a project's tests; returns the process exit code. */
export async function functionTest(
  target: string | undefined,
  options: FunctionTestOptions = {}
): Promise<number> {
  // A `js` or `starlark` function has no toolchain project. It still has
  // server cases, so it is testable — just not natively, because there is
  // nothing to compile and no mock host to compile against.
  const source = sourceFunctionTarget(target);
  if (source) {
    if (!options.server) {
      console.error(
        `${path.basename(source.dir)} is a ${source.language} function: there is no native ` +
          `test step for it (nothing to compile, and no mock host).\n` +
          `Run its cases against a server with \`--server\`, or invoke it directly with ` +
          `\`raisindb function run\`.`
      );
      return 2;
    }
    const cases = loadCasesFrom(sourceCasesFile(source.dir));
    if (cases.length === 0) {
      console.error(
        `No server cases: ${sourceCasesFile(source.dir)} is missing or empty.\n` +
          'Add [{ "input": {}, "expect": {} }] to it. The file is hidden so it is ' +
          'not uploaded as content.'
      );
      return 2;
    }
    return await reportServerCases({ dir: source.dir } as WasmProject, cases, options);
  }

  const project = soleProject(target);

  if (!options.server) {
    return (await runNativeTests(project)) === 0 ? 0 : 1;
  }

  const cases = loadServerCases(project.dir);
  if (cases.length === 0) {
    console.error(
      `No server cases: ${serverCasesFile(project.dir)} is missing or empty.\n` +
        'Add [{ "handler": "default", "input": {}, "expect": {} }] to it.'
    );
    return 2;
  }

  return await reportServerCases(project, cases, options);
}

/** Run server cases and print the result table. Shared by both target kinds. */
async function reportServerCases(
  project: WasmProject,
  cases: ServerCase[],
  options: FunctionTestOptions
): Promise<number> {
  console.log(`Running ${cases.length} server case(s) for ${path.basename(project.dir)}...`);
  const results = await runServerCases(project, cases, {
    server: typeof options.server === 'string' ? options.server : undefined,
    repo: options.repo,
    branch: options.branch,
    timeoutMs: parseTimeout(options.timeout),
  });

  for (const result of results) {
    const name = `case ${result.index + 1}${result.handler ? ` (${result.handler})` : ''}`;
    const timing = result.durationMs !== undefined ? ` [${result.durationMs} ms]` : '';
    if (result.passed) console.log(`+ ${name}${timing}`);
    else console.log(`x ${name}${timing}: ${result.detail}`);
  }

  const failed = results.filter((r) => !r.passed).length;
  console.log('');
  console.log(`Summary: ${results.length - failed} passed, ${failed} failed`);
  return failed > 0 ? 1 : 0;
}
