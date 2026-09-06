import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import fs from 'fs';
import os from 'os';
import path from 'path';
import { runWasmDoctor } from './doctor.js';
import { validateWasmFunctions } from './package-check.js';
import { MAX_ARTIFACT_BYTES } from './types.js';

let root: string;

function write(rel: string, content: string | Buffer): void {
  const full = path.join(root, rel);
  fs.mkdirSync(path.dirname(full), { recursive: true });
  fs.writeFileSync(full, content);
}

function node(name: string, entryFile: string, language = 'wasm'): void {
  write(
    `content/functions/lib/demo/${name}/.node.yaml`,
    `node_type: raisin:Function\nproperties:\n  name: ${name}\n  language: ${language}\n  entry_file: ${entryFile}\n`
  );
}

/** A rust project registering `default` and `shout`, with a built artifact. */
function rustProject(): void {
  write(
    'wasm/demo/greet/raisin.build.yaml',
    'lang: rust\nnode_dir: ../../../content/functions/lib/demo/greet\n'
  );
  write('wasm/demo/greet/Cargo.toml', '[package]\nname = "greet"\n');
  write(
    'wasm/demo/greet/src/lib.rs',
    `#[raisin_sdk::handler]
pub fn greet(i: Input) -> Result<Output> { todo!() }

#[raisin_sdk::handler(name = "shout")]
pub fn shout(i: Input) -> Result<Output> { todo!() }

raisin_sdk::export!(greet, shout);
`
  );
  write('content/functions/lib/demo/greet/main.wasm', Buffer.from([0, 0x61, 0x73, 0x6d]));
}

/** Never probe the machine's toolchains: the checks under test are about the
 * package, and a CI box without cargo must not turn them red. */
function doctor(strict = false) {
  return runWasmDoctor(root, { strict, toolchains: false });
}

function codes(strict = false): string[] {
  return doctor(strict).findings.map((f) => `${f.severity}:${f.code}`);
}

beforeEach(() => {
  root = fs.mkdtempSync(path.join(os.tmpdir(), 'raisin-doctor-'));
  write('manifest.yaml', 'name: demo\nversion: 0.1.0\n');
});

afterEach(() => {
  fs.rmSync(root, { recursive: true, force: true });
});

describe('runWasmDoctor', () => {
  it('is clean when the entry_file names a registered handler', () => {
    rustProject();
    node('greet', 'main.wasm');
    node('greet-shout', '../greet/main.wasm:shout');

    const report = doctor();
    expect(report.findings.filter((f) => f.severity === 'error')).toEqual([]);
    expect(report.exitCode).toBe(0);
  });

  it('catches a handler name the project never registers', () => {
    rustProject();
    node('greet', 'main.wasm:whisper');

    const report = doctor();
    const finding = report.findings.find((f) => f.code === 'WASM_HANDLER_NOT_REGISTERED');
    expect(finding?.severity).toBe('error');
    expect(finding?.message).toMatch(/registers: default, shout/);
    expect(report.exitCode).toBe(1);
  });

  it('refuses an entry_file that escapes the functions workspace', () => {
    rustProject();
    node('greet', 'main.wasm');
    node('evil', '../../../../../../etc/passwd.wasm:default');

    expect(codes()).toContain('error:WASM_ENTRY_FILE_ESCAPES');
    expect(doctor().exitCode).toBe(1);
  });

  it('warns rather than errors when the artifact has not been built yet', () => {
    rustProject();
    fs.rmSync(path.join(root, 'content/functions/lib/demo/greet/main.wasm'));
    node('greet', 'main.wasm');

    const report = doctor();
    expect(report.findings.some((f) => f.code === 'WASM_ARTIFACT_MISSING')).toBe(true);
    expect(report.exitCode).toBe(0);
    expect(doctor(true).exitCode).toBe(1);
  });

  it('exits 2 when there is nothing to look at', () => {
    expect(doctor().exitCode).toBe(2);
  });

  it('reports an unparseable raisin.build.yaml as an error', () => {
    write('wasm/demo/x/raisin.build.yaml', 'lang: cobol\nnode_dir: .\n');
    expect(codes()).toContain('error:WASM_BUILD_FILE_INVALID');
  });
});

describe('validateWasmFunctions (package validate)', () => {
  it('passes a package whose artifact is present', () => {
    rustProject();
    node('greet', 'main.wasm');
    expect(validateWasmFunctions(root)).toEqual({});
  });

  it('fails a package whose entry_file target was never built', () => {
    node('greet', 'main.wasm');
    const results = validateWasmFunctions(root);
    const file = Object.keys(results)[0];
    expect(results[file].errors[0].error_code).toBe('WASM_ARTIFACT_MISSING');
    expect(results[file].success).toBe(false);
  });

  it('fails a package whose entry_file escapes the functions workspace', () => {
    node('evil', '../../../../../../etc/passwd.wasm');
    const results = validateWasmFunctions(root);
    expect(Object.values(results)[0].errors[0].error_code).toBe('WASM_ENTRY_FILE_ESCAPES');
  });

  it('fails an artifact bigger than the server cap', () => {
    node('fat', 'main.wasm');
    write('content/functions/lib/demo/fat/main.wasm', Buffer.alloc(MAX_ARTIFACT_BYTES + 1));
    const results = validateWasmFunctions(root);
    expect(Object.values(results)[0].errors[0].error_code).toBe('WASM_ARTIFACT_TOO_LARGE');
  });

  it('ignores non-wasm functions entirely', () => {
    node('legacy', 'index.js', 'javascript');
    expect(validateWasmFunctions(root)).toEqual({});
  });
});
