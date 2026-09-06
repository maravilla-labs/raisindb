/**
 * Running guest toolchains, and reporting what is installed.
 *
 * The one non-obvious rule here is the **env scrub**. A wasm guest build must
 * not inherit cargo settings from whatever shell (or wrapper, or parent cargo
 * invocation) started the CLI: this repository's `.cargo/config.toml` sets
 * `split-debuginfo` for the host target and an inherited `CARGO_TARGET_DIR`
 * would drop a wasm artifact into a shared, host-target directory. Both make a
 * guest build fail in ways whose message names none of the above — see the
 * rules in `crates/raisin-server/build.rs` for the same problem on the other
 * side of the fence.
 */

import { spawn, spawnSync } from 'child_process';
import type { WasmLang } from './types.js';

/**
 * Environment variables that must never reach a guest toolchain.
 *
 * Scrubbed, not overridden: an empty value is itself meaningful to cargo.
 */
export const SCRUBBED_ENV_VARS = [
  'CARGO_TARGET_DIR',
  'RUSTFLAGS',
  'CARGO_ENCODED_RUSTFLAGS',
  'CARGO_BUILD_RUSTFLAGS',
  'CARGO_BUILD_TARGET',
];

/** `process.env` with the host-build variables removed. */
export function scrubbedEnv(base: NodeJS.ProcessEnv = process.env): NodeJS.ProcessEnv {
  const env = { ...base };
  for (const name of SCRUBBED_ENV_VARS) delete env[name];
  return env;
}

/** Result of one toolchain command. */
export interface RunResult {
  code: number;
  durationMs: number;
}

/**
 * Run a build command in `cwd`, streaming its output to this terminal.
 *
 * The command is handed to the platform shell verbatim so `raisin.build.yaml`
 * can carry a pipeline or a multi-line YAML scalar, exactly as the examples do.
 */
export function runCommand(command: string, cwd: string): Promise<RunResult> {
  const started = Date.now();
  return new Promise((resolve, reject) => {
    const child = spawn(command, {
      cwd,
      shell: true,
      stdio: 'inherit',
      env: scrubbedEnv(),
    });
    child.on('error', reject);
    child.on('close', (code) => resolve({ code: code ?? 1, durationMs: Date.now() - started }));
  });
}

/** A probed toolchain binary. */
export interface ToolStatus {
  /** Binary name as invoked. */
  name: string;
  /** First line of its version output, or null when it is not installed. */
  version: string | null;
  /** False when the tool is optional and its absence is only a hint. */
  required: boolean;
}

/** Full stdout of a command, or null when it cannot be run. */
export function output(name: string, args: string[]): string | null {
  try {
    const result = spawnSync(name, args, { encoding: 'utf-8', env: scrubbedEnv() });
    if (result.error || result.status !== 0) return null;
    return `${result.stdout || result.stderr}`;
  } catch {
    return null;
  }
}

/** Probe one binary's version — the FIRST line of its output. */
export function probe(name: string, args: string[] = ['--version']): string | null {
  const out = output(name, args);
  return out === null ? null : out.trim().split('\n')[0] || null;
}

/**
 * True when the rust `wasm32-wasip2` target is installed.
 *
 * Reads the WHOLE listing, not `probe`'s first line — the installed targets are
 * one per line and `wasm32-wasip2` is rarely the first of them.
 */
export function hasWasip2Target(): boolean {
  const out = output('rustup', ['target', 'list', '--installed']);
  return out !== null && /\bwasm32-wasip2\b/.test(out);
}

/** The toolchain a language needs, plus the optional extras `doctor` reports. */
export function toolchainFor(lang: WasmLang): ToolStatus[] {
  switch (lang) {
    case 'rust':
      return [
        { name: 'cargo', version: probe('cargo'), required: true },
        {
          name: 'rustup target wasm32-wasip2',
          version: hasWasip2Target() ? 'installed' : null,
          required: true,
        },
      ];
    case 'go':
      return [
        { name: 'tinygo', version: probe('tinygo', ['version']), required: true },
        { name: 'go', version: probe('go', ['version']), required: true },
      ];
    case 'ts':
      return [
        { name: 'node', version: probe('node'), required: true },
        { name: 'npm', version: probe('npm'), required: true },
      ];
  }
}

/**
 * `jco`, used by `doctor` to read a built component's world.
 *
 * Optional on purpose: a machine without it must still get a useful report,
 * with the world check reported as skipped rather than failed.
 */
export function jcoVersion(): string | null {
  return probe('jco', ['--version']);
}

/** Inspect a component's WIT world with `jco wit`. Null when jco is absent. */
export function jcoWit(artifact: string): string | null {
  if (!jcoVersion()) return null;
  const result = spawnSync('jco', ['wit', artifact], { encoding: 'utf-8', env: scrubbedEnv() });
  if (result.error || result.status !== 0) return null;
  return result.stdout;
}
