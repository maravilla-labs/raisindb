/**
 * Pieces every wasm-function scaffold shares: the substitution variables, the
 * `.node.yaml` for the Function node, the `raisin.build.yaml` for the toolchain
 * project, and the identifier spellings each language wants for a handler name.
 */

import type { FileEntry } from '../types.js';
import type { WasmLang } from '../../wasm-fn/types.js';

/** How a scaffolded project depends on the guest SDK. */
export interface SdkRef {
  /**
   * How a scaffolded project reaches the guest SDK.
   *
   * * `path` — inside this monorepo (or a checkout of it).
   * * `git`  — everywhere else: the SDKs live in the public `raisindb` repo
   *   and are pinned to a release tag. This is the default outside the
   *   monorepo because the Rust crate is NOT on crates.io (`publish = false`)
   *   and the Go module is a subdirectory module; emitting a bare version
   *   requirement produced a project that could not resolve its dependency.
   * * `version` — a registry requirement, for when the SDKs are published.
   */
  kind: 'path' | 'git' | 'version';
  /** The path, the git tag, or the version requirement. */
  value: string;
}

/** The public repository the guest SDKs are published from. */
export const SDK_GIT_URL = 'https://github.com/maravilla-labs/raisindb';

/** Substitution variables for a scaffolded wasm function. */
export interface WasmFnVars {
  /** Function slug — the node name and the project directory name. */
  name: string;
  /** Namespace segment under `content/functions/lib/`. */
  ns: string;
  /** Handler name the scaffolded node's `entry_file` selects. */
  handler: string;
  /** Human title for the node. */
  title: string;
  /** One-line description. */
  description: string;
  /** Guest language. */
  lang: WasmLang;
  /** How to reach the SDK from the scaffolded project. */
  sdk: SdkRef;
  /** Path from the toolchain project to the Function node directory. */
  nodeDirRel: string;
}

/** Title-case a kebab slug, for node titles. */
export function titleCase(slug: string): string {
  return slug
    .split(/[-_]/g)
    .filter(Boolean)
    .map((s) => s.charAt(0).toUpperCase() + s.slice(1))
    .join(' ');
}

/** `on-order` → `on_order`; the default handler's function is `handler`. */
export function snakeIdent(handler: string): string {
  if (handler === 'default') return 'handler';
  return handler.replace(/[^A-Za-z0-9]+/g, '_').replace(/^_+|_+$/g, '') || 'handler';
}

/** `on-order` → `onOrder`; the default handler's function is `handler`. */
export function camelIdent(handler: string): string {
  if (handler === 'default') return 'handler';
  const parts = handler.split(/[^A-Za-z0-9]+/).filter(Boolean);
  if (parts.length === 0) return 'handler';
  return parts[0] + parts.slice(1).map((p) => p.charAt(0).toUpperCase() + p.slice(1)).join('');
}

/** `entry_file` value for a node whose artifact sits in its own directory. */
export function entryFileFor(handler: string, artifact = 'main.wasm'): string {
  return handler === 'default' ? artifact : `${artifact}:${handler}`;
}

/** The Function node declaration that ships the artifact. */
export function nodeYaml(v: WasmFnVars): string {
  return `node_type: raisin:Function
properties:
  name: ${v.name}
  title: ${v.title}
  description: >
    ${v.description}
  enabled: true
  language: wasm
  execution_mode: both
  # The artifact lives beside this file. A suffix selects a handler:
  # \`main.wasm:on-order\`; a bare \`main.wasm\` means the handler named
  # "default". A sibling node may point at THIS artifact with
  # \`../${v.name}/main.wasm:<handler>\` — one component, many functions.
  entry_file: ${entryFileFor(v.handler)}
  version: 1
  resource_limits:
    timeout_ms: 5000
    # Bytes, not megabytes: \`ResourceLimits\` has \`max_memory_bytes\` and
    # ignores anything it does not know, so a wrong key is silently the
    # 128 MiB default.
    max_memory_bytes: 67108864
  network_policy:
    http_enabled: false
  input_schema:
    type: object
    additionalProperties: false
    properties:
      name:
        type: string
        description: Who to greet
    required: [name]
  output_schema:
    type: object
    properties:
      greeting:
        type: string
      handler:
        type: string
`;
}

/** Default build command per language, written into `raisin.build.yaml`. */
function buildCommand(v: WasmFnVars): string {
  switch (v.lang) {
    case 'rust':
      return 'cargo build --release --target wasm32-wasip2';
    case 'go':
      return `>-
  tinygo build -target=wasip2 -o main.wasm
  --wit-package ${v.sdk.kind === 'path' ? `${v.sdk.value}/wit` : './wit'}
  --wit-world function .`;
    case 'ts':
      return 'npm run build';
  }
}

/** Where the toolchain writes, when it is not the project directory. */
function buildOutput(v: WasmFnVars): string {
  if (v.lang !== 'rust') return '';
  return `output: target/wasm32-wasip2/release/${v.name.replace(/-/g, '_')}.wasm\n`;
}

/** `raisin.build.yaml` — how `raisindb function build` turns source into bytes. */
export function buildYaml(v: WasmFnVars): string {
  return `# How \`raisindb function build\` turns this project into the Function node's
# artifact. ONE artifact can back several Function nodes: add a second handler
# with \`raisindb create function <other> --lang ${v.lang} --into ${v.name}\`.
lang: ${v.lang}
node_dir: ${v.nodeDirRel}
artifact: main.wasm
handlers:
  - ${v.handler}
command: ${buildCommand(v)}
${buildOutput(v)}`;
}

/** The `.node.yaml` + `raisin.build.yaml` pair every language scaffold emits. */
export function commonFiles(v: WasmFnVars, nodePath: string, projectPath: string): FileEntry[] {
  return [
    { path: `${nodePath}/.node.yaml`, content: nodeYaml(v) },
    { path: `${projectPath}/raisin.build.yaml`, content: buildYaml(v) },
  ];
}
