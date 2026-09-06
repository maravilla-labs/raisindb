/**
 * AssemblyScript guest scaffold.
 *
 * Unlike the other guests this one needs THREE build steps, because `asc`
 * emits a core module: compile, attach the WIT, wrap as a component. The
 * scaffold therefore carries `wit/` in the project (like Go) and lists
 * `wasm-tools` as a required toolchain.
 */

import type { FileEntry } from '../types.js';
import { RAISIN_WIT } from './wit.js';
import type { WasmFnVars } from './shared.js';

/** Published range for `@raisindb/function-assemblyscript`. */
const AS_SDK_RANGE = '^0.1.0';

function packageJson(v: WasmFnVars): string {
  // npm cannot install a package from a git SUBDIRECTORY, and this SDK lives
  // in `sdks/assemblyscript` of the monorepo — so outside a checkout it comes
  // from the registry, exactly like the TypeScript SDK. A git ref here would
  // resolve to the repository root and fail to find the package.
  const sdk = v.sdk.kind === 'path' ? `"file:${v.sdk.value}"` : `"${AS_SDK_RANGE}"`;
  return `{
  "name": "${v.name}",
  "private": true,
  "type": "module",
  "scripts": {
    "build": "asc assembly/index.ts -o build/guest.core.wasm --runtime stub --exportRuntime --optimize --use abort=",
    "check": "asc assembly/index.ts --noEmit"
  },
  "devDependencies": {
    "assemblyscript": "^0.28.20"
  },
  "dependencies": {
    "@raisindb/function-assemblyscript": ${sdk}
  }
}
`;
}

function entry(v: WasmFnVars): string {
  // A relative path INTO node_modules, not the bare package name.
  //
  // `asc` does not resolve a bare scoped import through node_modules — it maps
  // it to `~lib/@raisindb/function-assemblyscript.ts` and fails, with or
  // without `--lib node_modules`. Pointing at the installed package directly
  // works for both cases: npm puts it in the same place whether the dependency
  // is the monorepo `file:` link or a registry install.
  const sdkPath = '../node_modules/@raisindb/function-assemblyscript/assembly/index';
  return `// ${v.name} — a RaisinDB function in AssemblyScript.
//
// The component exports ONE function. The node's \`entry_file\` suffix picks the
// handler, so routing is an ordinary comparison — add a case and a Function
// node pointing at \`main.wasm:<name>\` to serve several from one artifact.

import { run, log, cabi_realloc, unknownHandler, nodes } from "${sdkPath}";

/** The "${v.handler}" handler. Takes and returns JSON TEXT. */
function ${v.handler === 'default' ? 'greet' : v.handler}(input: string): string {
  log.info("${v.name} running");

  // Every raisin.* method is available and returns raw JSON.
  const children = nodes.getChildren("content", "/pages", 50);

  return '{"greeting":"hello","children":' + children + '}';
}

function route(name: string, input: string): string {
  if (name == "${v.handler}") return ${v.handler === 'default' ? 'greet' : v.handler}(input);
  return unknownHandler(name, "${v.handler}");
}

// Both exports are looked up BY NAME by \`wasm-tools component new\`.
export function handler(np: i32, nl: i32, ip: i32, il: i32): i32 {
  return run(np, nl, ip, il, route);
}
export { cabi_realloc };
`;
}

function readme(v: WasmFnVars): string {
  return `# ${v.name}

An AssemblyScript RaisinDB function, compiled to a WebAssembly component.

\`\`\`bash
npm install
raisindb function doctor .        # checks asc + wasm-tools
raisindb function build .         # asc -> component embed -> component new
raisindb function run . --input '{"name":"Ada"}'
\`\`\`

## Why three build steps

AssemblyScript does not implement the Component Model, so \`asc\` produces a
CORE MODULE. \`wasm-tools component embed\` attaches the WIT in \`wit/\` and
\`component new\` wraps the result. \`raisindb function build\` runs all three.

## Handlers

\`handler\` and \`cabi_realloc\` must both be exported from \`assembly/index.ts\`
— they are resolved by name. Add a handler by extending \`route\` and creating a
Function node whose \`entry_file\` is \`main.wasm:<name>\`; both nodes then share
this one artifact.

## JSON

Handlers take and return JSON **text**. AssemblyScript has no built-in JSON and
bundling one would make every artifact pay for it; use
[\`json-as\`](https://github.com/JairusSW/as-json) if you want typed
(de)serialisation.
`;
}

export function assemblyScriptFiles(v: WasmFnVars, projectPath: string): FileEntry[] {
  return [
    { path: `${projectPath}/package.json`, content: packageJson(v) },
    { path: `${projectPath}/assembly/index.ts`, content: entry(v) },
    // `component embed` reads the contract from here, so it must be IN the
    // project — the SDK's copy is not on the build's path.
    { path: `${projectPath}/wit/raisin-function.wit`, content: RAISIN_WIT },
    { path: `${projectPath}/README.md`, content: readme(v) },
    { path: `${projectPath}/.gitignore`, content: 'node_modules/\nbuild/\n' },
  ];
}
