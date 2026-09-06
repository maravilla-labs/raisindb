/**
 * TypeScript/JavaScript guest scaffold.
 *
 * The source is an ordinary QuickJS-style function — same `globalThis.raisin`,
 * same `console`, same per-method error conventions — and componentizing it is
 * a build step, not a rewrite. `raisin-wasm-build` (from
 * `@raisindb/function-wasm`) wraps `jco componentize` and generates the entry
 * that routes the WIT `handler(name, input)` export to this module's exports.
 */

import type { FileEntry } from '../types.js';
import { camelIdent, type WasmFnVars } from './shared.js';

function packageJson(v: WasmFnVars): string {
  const sdk = v.sdk.kind === 'path' ? `file:${v.sdk.value}` : `^${v.sdk.value}`;
  return `${JSON.stringify(
    {
      name: v.name,
      version: '0.1.0',
      type: 'module',
      private: true,
      description: v.description,
      scripts: {
        build: 'raisin-wasm-build src/index.js --out main.wasm',
        test: 'vitest run',
      },
      devDependencies: {
        '@raisindb/function-wasm': sdk,
        vitest: '^3.2.4',
      },
    },
    null,
    2
  )}\n`;
}

function indexJs(v: WasmFnVars): string {
  const ident = camelIdent(v.handler);
  return `/**
 * ${v.name} — a RaisinDB function compiled to a WebAssembly component.
 *
 * Note what is NOT here: no import of the SDK, no host plumbing, no
 * registration call. Every exported function is a handler, addressed by name
 * from a Function node's \`entry_file\`:
 *
 *   entry_file: main.wasm                     -> "default" -> handler
 *   entry_file: main.wasm:shout               -> "shout"   -> shout
 *   entry_file: ../${v.name}/main.wasm:shout  (from a sibling node directory)
 */

/** ${v.description} */
export async function ${ident}(input) {
  const name = input && input.name;
  if (!name) throw new Error('input.name is required');
  console.log('greeting', name);
  return { greeting: \`Hello, \${name}!\`, handler: '${v.handler}' };
}
`;
}

function testJs(v: WasmFnVars): string {
  return `// Native tests: no wasm, no jco, no server. The mock host stands in for the
// WIT gateway, so the same source that ships as a component is exercised here
// in milliseconds.

import { beforeEach, describe, expect, it } from 'vitest';
import { createHandler } from '@raisindb/function-wasm';
import { createMockHost } from '@raisindb/function-wasm/testing';
import * as fn from '../src/index.js';

const handler = createHandler(fn);

let mock;
beforeEach(() => {
  mock = createMockHost({ context: { tenant_id: 'default', branch: 'main' } });
  mock.install();
});

describe('${v.name}', () => {
  it('greets', async () => {
    const out = JSON.parse(await handler('${v.handler}', JSON.stringify({ name: 'Ada' })));

    expect(out.greeting).toBe('Hello, Ada!');
    expect(mock.logs[0]).toEqual({ level: 'info', message: 'greeting Ada' });
  });

  it('rejects a missing name', async () => {
    await expect(handler('${v.handler}', '{}')).rejects.toThrow('input.name is required');
  });
});
`;
}

function readme(v: WasmFnVars): string {
  return `# ${v.title}

JavaScript guest for the \`${v.name}\` RaisinDB function, componentized with
jco / ComponentizeJS.

    raisindb function doctor .      # toolchain, entry_file, handler names
    npm install && npm test         # native tests, no server
    raisindb function build .       # jco componentize + copy to ${v.nodeDirRel}

Components are 8–15 MiB (a whole JS engine), against a 32 MiB server cap — so
prefer ONE artifact with several handlers over one artifact per function:

    raisindb create function ${v.name}-shout --lang ts --into ${v.name} --handler shout

Limits worth knowing before you write code: there is no \`setTimeout\` and no
native \`fetch\`. Use \`await\` and \`raisin.http.*\`.
`;
}

/** Every file the TypeScript scaffold writes, under `projectPath`. */
export function tsFiles(v: WasmFnVars, projectPath: string): FileEntry[] {
  return [
    { path: `${projectPath}/package.json`, content: packageJson(v) },
    { path: `${projectPath}/src/index.js`, content: indexJs(v) },
    { path: `${projectPath}/test/${v.name}.test.js`, content: testJs(v) },
    {
      path: `${projectPath}/tests/server.json`,
      content: `${JSON.stringify(
        [{ handler: v.handler, input: { name: 'Ada' }, expect: { greeting: 'Hello, Ada!' } }],
        null,
        2
      )}\n`,
    },
    { path: `${projectPath}/.gitignore`, content: 'node_modules/\nmain.wasm\n' },
    { path: `${projectPath}/README.md`, content: readme(v) },
  ];
}
