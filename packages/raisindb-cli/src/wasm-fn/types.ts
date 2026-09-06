/**
 * Shared types for the WebAssembly function developer loop
 * (`raisindb create function`, `raisindb function build|doctor`).
 *
 * The layout every command assumes — the one `raisindb create function`
 * scaffolds — keeps toolchain SOURCE out of the package content tree, because
 * `sync` maps every non-YAML file under `content/` to a node (see
 * `sync/mapping.ts`). Only the built artifact lives under `content/`:
 *
 * ```
 * my-package/
 *   content/functions/lib/<ns>/<name>/.node.yaml   language: wasm
 *   content/functions/lib/<ns>/<name>/main.wasm    the only thing that ships
 *   wasm/<ns>/<name>/raisin.build.yaml             how to build it
 *   wasm/<ns>/<name>/{Cargo.toml|go.mod|package.json, src/…}
 *   .rapignore                                     wasm/
 * ```
 */

/** Guest languages with a first-party SDK and a build lane. */
export type WasmLang = 'rust' | 'go' | 'ts' | 'assemblyscript';

/** The COMPILED languages `--lang` accepts, in help-text order. */
export const WASM_LANGS: WasmLang[] = ['rust', 'go', 'assemblyscript', 'ts'];

/**
 * Languages whose source IS the deliverable — no toolchain, no build step.
 *
 * `js` is the QuickJS runtime and `starlark` the Starlark one. They are here so
 * that `raisindb create function` is the way to start ANY function, not only a
 * WebAssembly one: the same command, the same node layout, the same
 * `function run`. What differs is only that there is nothing to compile, so
 * they have no `raisin.build.yaml` and their code lives under `content/`
 * (where, being the deliverable, it is exactly what should ship).
 */
export type SourceLang = 'js' | 'starlark';

/** The SOURCE languages `--lang` accepts, in help-text order. */
export const SOURCE_LANGS: SourceLang[] = ['js', 'starlark'];

/** Every language `raisindb create function` understands. */
export type FunctionLang = WasmLang | SourceLang;

/** True when the language compiles to an artifact rather than shipping source. */
export function isWasmLang(lang: string): lang is WasmLang {
  return (WASM_LANGS as string[]).includes(lang);
}

/** True when the language ships its source. */
export function isSourceLang(lang: string): lang is SourceLang {
  return (SOURCE_LANGS as string[]).includes(lang);
}

/** The `language` property a source language writes into `.node.yaml`. */
export const SOURCE_NODE_LANGUAGE: Record<SourceLang, string> = {
  js: 'javascript',
  starlark: 'starlark',
};

/** The entry file a source language scaffolds. */
export const SOURCE_ENTRY_FILE: Record<SourceLang, string> = {
  js: 'index.js',
  starlark: 'main.star',
};

/**
 * Server-side artifact cap (`[functions.wasm] max_artifact_bytes`, 32 MiB).
 *
 * Mirrored here so `doctor` and `package create` can refuse an artifact the
 * server would reject at upload. An operator may lower it; this is the default.
 */
export const MAX_ARTIFACT_BYTES = 33_554_432;

/** The handler name a bare `entry_file: main.wasm` selects. */
export const DEFAULT_HANDLER = 'default';

/** `raisin.build.yaml` — how one toolchain project becomes one artifact. */
export interface BuildSpec {
  /** Guest language; selects the default command and output path. */
  lang: WasmLang;
  /** Function node directory, relative to the build file. */
  node_dir: string;
  /** Artifact filename inside `node_dir`. Conventionally `main.wasm`. */
  artifact?: string;
  /** Build command, run in the project directory. Defaults per language. */
  command?: string;
  /** Built artifact path relative to the project. Defaults per language. */
  output?: string;
  /** Informational: the handler names this project registers. */
  handlers?: string[];
}

/** A resolved toolchain project: a `raisin.build.yaml` plus its defaults. */
export interface WasmProject {
  /** Absolute project directory (the one holding `raisin.build.yaml`). */
  dir: string;
  /** Absolute path of the build file. */
  buildFile: string;
  /** The parsed build file. */
  spec: BuildSpec;
  /** Absolute Function node directory the artifact is copied into. */
  nodeDir: string;
  /** Absolute destination path (`nodeDir/artifact`). */
  artifactPath: string;
  /** Absolute path the toolchain writes. */
  outputPath: string;
  /** The command actually run. */
  command: string;
}

/** A `raisin:Function` node declared by a `.node.yaml` in the package. */
export interface FunctionNode {
  /** Absolute path of the `.node.yaml`. */
  file: string;
  /** Absolute directory the node lives in. */
  dir: string;
  /** Node name (`properties.name`, else the directory name). */
  name: string;
  /** Declared `properties.language`, verbatim. */
  language: string;
  /** Declared `properties.entry_file`, verbatim (may be absent). */
  entryFile: string;
  /** Absolute artifact path the entry file resolves to, or null when unset. */
  artifactPath: string | null;
  /** Handler name the entry file selects (`default` when unsuffixed). */
  handler: string;
  /** True when the resolved path escapes the functions workspace root. */
  escapes: boolean;
}
