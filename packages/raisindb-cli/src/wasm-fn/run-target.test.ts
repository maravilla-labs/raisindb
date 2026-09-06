import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import fs from 'fs';
import os from 'os';
import path from 'path';
import {
  planRun,
  resolveInput,
  resolveRunTarget,
  workspaceLocation,
} from './run-target.js';

let root: string;

function write(rel: string, content: string): string {
  const full = path.join(root, rel);
  fs.mkdirSync(path.dirname(full), { recursive: true });
  fs.writeFileSync(full, content);
  return full;
}

/** A Function node + its artifact, plus the project that builds it. */
function fixture(name: string, entryFile = 'main.wasm'): void {
  write(
    `content/functions/lib/demo/${name}/.node.yaml`,
    `node_type: raisin:Function\nproperties:\n  name: ${name}\n  language: wasm\n  entry_file: ${entryFile}\n`
  );
  write(`wasm/demo/${name}/raisin.build.yaml`, `lang: rust\nnode_dir: ../../../content/functions/lib/demo/${name}\n`);
}

beforeEach(() => {
  root = fs.mkdtempSync(path.join(os.tmpdir(), 'raisin-run-'));
  write('manifest.yaml', 'name: demo\nversion: 0.1.0\n');
});

afterEach(() => {
  fs.rmSync(root, { recursive: true, force: true });
});

describe('workspaceLocation', () => {
  it('splits a content path into workspace and node path', () => {
    fs.mkdirSync(path.join(root, 'content/functions/lib/demo/greet'), { recursive: true });
    expect(
      workspaceLocation(root, path.join(root, 'content/functions/lib/demo/greet/main.wasm'))
    ).toEqual({ workspace: 'functions', nodePath: 'lib/demo/greet/main.wasm' });
  });

  it('refuses an artifact outside the content tree', () => {
    expect(() => workspaceLocation(root, path.join(root, '..', 'elsewhere.wasm'))).toThrow(
      /outside the package content tree/
    );
  });
});

describe('resolveRunTarget', () => {
  it('resolves from the Function node directory', () => {
    fixture('greet');
    const target = resolveRunTarget(path.join(root, 'content/functions/lib/demo/greet'));
    expect(target.node.name).toBe('greet');
    expect(target.handler).toBe('default');
    expect(target.handlerOverridden).toBe(false);
    expect(target.artifactPath).toBe(
      path.join(root, 'content/functions/lib/demo/greet/main.wasm')
    );
  });

  it('resolves from the toolchain project directory', () => {
    fixture('greet');
    const target = resolveRunTarget(path.join(root, 'wasm/demo/greet'));
    expect(target.node.name).toBe('greet');
    expect(target.project?.dir).toBe(path.join(root, 'wasm/demo/greet'));
  });

  it('refuses to guess between two functions', () => {
    fixture('greet');
    fixture('shout', 'main.wasm:shout');
    expect(() => resolveRunTarget(root)).toThrow(/2 wasm functions/);
  });

  it('--handler picks the node that declares it', () => {
    fixture('greet');
    fixture('shout', 'main.wasm:shout');
    const target = resolveRunTarget(root, { handler: 'shout' });
    expect(target.node.name).toBe('shout');
    expect(target.handlerOverridden).toBe(false);
  });

  it('allows a handler no node declares, and marks it overridden', () => {
    fixture('greet');
    const target = resolveRunTarget(root, { handler: 'whisper' });
    expect(target.handler).toBe('whisper');
    expect(target.handlerOverridden).toBe(true);
  });

  it('refuses an entry_file that escapes the functions workspace', () => {
    fixture('greet', '../../../../../etc/main.wasm');
    expect(() => resolveRunTarget(path.join(root, 'content/functions/lib/demo/greet'))).toThrow(
      /outside the functions workspace/
    );
  });

  it('explains an empty package rather than resolving nothing', () => {
    expect(() => resolveRunTarget(root)).toThrow(/No `language: wasm` Function node/);
  });
});

describe('planRun', () => {
  const base = {
    handlerOverridden: false,
    functionExists: true,
    serverHash: 'abc',
    localHash: 'abc',
  };

  it('invokes when the server holds the same bytes', () => {
    expect(planRun(base).mode).toBe('invoke');
  });

  it('uploads when the hashes differ', () => {
    expect(planRun({ ...base, serverHash: 'other' }).mode).toBe('run-file');
  });

  it('uploads when the server recorded no hash', () => {
    expect(planRun({ ...base, serverHash: null }).mode).toBe('run-file');
  });

  it('uploads when the function is not deployed', () => {
    expect(planRun({ ...base, functionExists: false }).mode).toBe('run-file');
  });

  it('uploads for a handler override, since invoke would run the node handler', () => {
    expect(planRun({ ...base, handlerOverridden: true }).mode).toBe('run-file');
  });
});

describe('resolveInput', () => {
  it('defaults to an empty object', () => {
    expect(resolveInput(undefined, undefined)).toEqual({});
  });

  it('parses inline JSON', () => {
    expect(resolveInput('{"name":"Ada"}', undefined)).toEqual({ name: 'Ada' });
  });

  it('reads a file', () => {
    const file = write('input.json', '{"name":"Grace"}');
    expect(resolveInput(undefined, file)).toEqual({ name: 'Grace' });
  });

  it('refuses both at once', () => {
    expect(() => resolveInput('{}', 'x.json')).toThrow(/mutually exclusive/);
  });

  it('names the source of invalid JSON', () => {
    expect(() => resolveInput('{oops', undefined)).toThrow(/--input: not valid JSON/);
  });
});
