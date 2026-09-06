import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import fs from 'fs';
import os from 'os';
import path from 'path';
import { registeredHandlers } from './handlers.js';
import { loadProject } from './discover.js';
import type { WasmProject } from './types.js';

let root: string;

function write(rel: string, content: string): string {
  const full = path.join(root, rel);
  fs.mkdirSync(path.dirname(full), { recursive: true });
  fs.writeFileSync(full, content);
  return full;
}

/** A project whose sources live under `wasm/p`. */
function project(lang: string): WasmProject {
  const build = write('wasm/p/raisin.build.yaml', `lang: ${lang}\nnode_dir: ../../content\n`);
  return loadProject(build);
}

beforeEach(() => {
  root = fs.mkdtempSync(path.join(os.tmpdir(), 'raisin-handlers-'));
});

afterEach(() => {
  fs.rmSync(root, { recursive: true, force: true });
});

describe('registeredHandlers — rust', () => {
  it('reads the export! list and each handler attribute', () => {
    write(
      'wasm/p/src/lib.rs',
      `
#[raisin_sdk::handler]
pub fn greet(input: Input) -> Result<Output> { todo!() }

#[raisin_sdk::handler(name = "shout")]
pub fn shout(input: Input) -> Result<Output> { todo!() }

raisin_sdk::export!(greet, shout);
`
    );
    expect(registeredHandlers(project('rust')).names).toEqual(['default', 'shout']);
  });

  it('does not count a handler the crate never exports', () => {
    write(
      'wasm/p/src/lib.rs',
      `
#[raisin_sdk::handler(name = "kept")]
pub fn kept(i: Input) -> Result<Output> { todo!() }

#[raisin_sdk::handler(name = "dropped")]
pub fn dropped(i: Input) -> Result<Output> { todo!() }

raisin_sdk::export!(kept);
`
    );
    expect(registeredHandlers(project('rust')).names).toEqual(['kept']);
  });

  it('says so, rather than reporting an empty set, when it cannot tell', () => {
    write('wasm/p/src/lib.rs', 'pub fn nothing() {}\n');
    const scan = registeredHandlers(project('rust'));
    expect(scan.names).toEqual([]);
    expect(scan.note).toMatch(/export!/);
  });
});

describe('registeredHandlers — go', () => {
  it('reads Handle and HandleDefault, ignoring test files', () => {
    write(
      'wasm/p/main.go',
      `package main

func init() {
	raisin.HandleDefault(greet)
	raisin.Handle("shout", shout)
}
`
    );
    write('wasm/p/main_test.go', 'package main\n\nfunc x() { raisin.Handle("only-in-tests", nil) }\n');
    expect(registeredHandlers(project('go')).names).toEqual(['default', 'shout']);
  });
});

describe('registeredHandlers — ts', () => {
  it('treats every export as a handler and `handler` as the default', () => {
    write(
      'wasm/p/src/index.js',
      `export async function handler(input) {}
export function shout(input) {}
export const onOrder = async (input) => {};
`
    );
    expect(registeredHandlers(project('ts')).names).toEqual(['default', 'onOrder', 'shout']);
  });
});
