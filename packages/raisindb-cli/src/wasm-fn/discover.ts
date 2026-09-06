/**
 * Locating the pieces of a wasm function project: the package root, the
 * `raisin.build.yaml` toolchain projects, and the `raisin:Function` nodes whose
 * `entry_file` points at a built artifact.
 *
 * Entry-file resolution mirrors the server
 * (`raisin-functions/src/execution/entry_file.rs`): `main.wasm:on-order` splits
 * into asset path and handler, a bare `main.wasm` means handler `"default"`,
 * and a parent-relative path (`../shared/main.wasm:on-order`) is legal only
 * while it stays inside the functions workspace.
 */

import fs from 'fs';
import path from 'path';
import yaml from 'yaml';
import {
  DEFAULT_HANDLER,
  WASM_LANGS,
  type BuildSpec,
  type FunctionNode,
  type WasmLang,
  type WasmProject,
} from './types.js';

/** Directories never worth descending into when scanning a package. */
const SKIP_DIRS = new Set(['node_modules', 'target', '.git', 'dist', 'build', '.svn']);

/** Walk up from `start` to the nearest directory holding a `manifest.yaml`. */
export function findPackageRoot(start: string): string | null {
  let dir = path.resolve(start);
  for (;;) {
    for (const name of ['manifest.yaml', 'manifest.yml']) {
      if (fs.existsSync(path.join(dir, name))) return dir;
    }
    const parent = path.dirname(dir);
    if (parent === dir) return null;
    dir = parent;
  }
}

/** The content base of a package: `content/` when present, else the root. */
export function contentBase(packageRoot: string): string {
  const nested = path.join(packageRoot, 'content');
  return fs.existsSync(nested) ? nested : packageRoot;
}

/** The functions workspace root — the boundary an `entry_file` may not cross. */
export function functionsRoot(packageRoot: string): string {
  return path.join(contentBase(packageRoot), 'functions');
}

/** Recursively collect files named `name`, skipping build/vendor directories. */
function findFiles(dir: string, name: string, out: string[] = []): string[] {
  let entries: fs.Dirent[];
  try {
    entries = fs.readdirSync(dir, { withFileTypes: true });
  } catch {
    return out;
  }
  for (const entry of entries) {
    if (entry.isDirectory()) {
      if (SKIP_DIRS.has(entry.name)) continue;
      findFiles(path.join(dir, entry.name), name, out);
    } else if (entry.name === name) {
      out.push(path.join(dir, entry.name));
    }
  }
  return out;
}

/** Default build command per language, run in the project directory. */
export function defaultCommand(lang: WasmLang, release: boolean): string {
  switch (lang) {
    case 'rust':
      return `cargo build ${release ? '--release ' : ''}--target wasm32-wasip2`;
    case 'go':
      return 'tinygo build -target=wasip2 -o main.wasm --wit-package ./wit --wit-world function .';
    case 'ts':
      return 'raisin-wasm-build src/index.js --out main.wasm';
    case 'assemblyscript':
      // Three steps, because AssemblyScript emits a CORE MODULE: compile, then
      // attach the WIT, then wrap it as a component. `wasm-tools` does the last
      // two and is a required toolchain for this language.
      return (
        // `npx`: asc is a local devDependency, not a global.
        'npx asc assembly/index.ts -o build/guest.core.wasm --runtime stub --exportRuntime ' +
        (release ? '--optimize ' : '') +
        '--use abort= && ' +
        'wasm-tools component embed wit build/guest.core.wasm -o build/guest.embed.wasm --world function && ' +
        'wasm-tools component new build/guest.embed.wasm -o main.wasm'
      );
  }
}

/** Crate name from a Cargo.toml, falling back to the directory name. */
function crateName(projectDir: string): string {
  const manifest = path.join(projectDir, 'Cargo.toml');
  try {
    const text = fs.readFileSync(manifest, 'utf-8');
    const match = text.match(/^\s*name\s*=\s*"([^"]+)"/m);
    if (match) return match[1];
  } catch {
    /* fall through */
  }
  return path.basename(projectDir);
}

/** Default path the toolchain writes, relative to the project directory. */
export function defaultOutput(lang: WasmLang, projectDir: string, release: boolean): string {
  switch (lang) {
    case 'rust':
      return path.join(
        'target',
        'wasm32-wasip2',
        release ? 'release' : 'debug',
        `${crateName(projectDir).replace(/-/g, '_')}.wasm`
      );
    case 'go':
    case 'ts':
    case 'assemblyscript':
      return 'main.wasm';
  }
}

/** Parse and resolve one `raisin.build.yaml`. Throws on an unusable spec. */
export function loadProject(buildFile: string, release = true): WasmProject {
  const dir = path.dirname(path.resolve(buildFile));
  let spec: BuildSpec;
  try {
    spec = yaml.parse(fs.readFileSync(buildFile, 'utf-8')) as BuildSpec;
  } catch (error) {
    throw new Error(
      `${buildFile}: not valid YAML — ${error instanceof Error ? error.message : String(error)}`
    );
  }
  if (!spec || typeof spec !== 'object') {
    throw new Error(`${buildFile}: expected a YAML mapping with lang/node_dir`);
  }
  if (!WASM_LANGS.includes(spec.lang)) {
    throw new Error(
      `${buildFile}: lang must be one of ${WASM_LANGS.join(', ')} (got ${JSON.stringify(spec.lang)})`
    );
  }
  if (!spec.node_dir) throw new Error(`${buildFile}: node_dir is required`);

  const artifact = spec.artifact || 'main.wasm';
  const nodeDir = path.resolve(dir, spec.node_dir);
  return {
    dir,
    buildFile: path.resolve(buildFile),
    spec,
    nodeDir,
    artifactPath: path.join(nodeDir, artifact),
    outputPath: path.resolve(dir, spec.output || defaultOutput(spec.lang, dir, release)),
    command: spec.command || defaultCommand(spec.lang, release),
  };
}

/**
 * Every toolchain project under `target`.
 *
 * `target` may be a `raisin.build.yaml`, a project directory, or any directory
 * above one (a package root, typically). Unparseable build files are returned
 * as failures rather than thrown, so `doctor` can report all of them at once.
 */
export function discoverProjects(
  target: string,
  release = true
): { projects: WasmProject[]; failures: { file: string; error: string }[] } {
  const resolved = path.resolve(target);
  let files: string[];
  if (fs.existsSync(resolved) && fs.statSync(resolved).isFile()) {
    files = [resolved];
  } else if (fs.existsSync(path.join(resolved, 'raisin.build.yaml'))) {
    files = [path.join(resolved, 'raisin.build.yaml')];
  } else {
    files = findFiles(resolved, 'raisin.build.yaml').sort();
  }

  const projects: WasmProject[] = [];
  const failures: { file: string; error: string }[] = [];
  for (const file of files) {
    try {
      projects.push(loadProject(file, release));
    } catch (error) {
      failures.push({ file, error: error instanceof Error ? error.message : String(error) });
    }
  }
  return { projects, failures };
}

/**
 * Split an `entry_file` into its asset path and handler name.
 *
 * `main.wasm:on-order` → `["main.wasm", "on-order"]`; a bare `main.wasm` →
 * handler `"default"`; a trailing bare `:` falls back to the default, matching
 * the server. A Windows-style drive letter is not a concern: entry files are
 * package-relative posix paths.
 */
export function splitEntryFile(entryFile: string): { asset: string; handler: string } {
  const trimmed = entryFile.trim();
  const colon = trimmed.lastIndexOf(':');
  if (colon <= 0) return { asset: trimmed, handler: DEFAULT_HANDLER };
  const handler = trimmed.slice(colon + 1).trim();
  return { asset: trimmed.slice(0, colon), handler: handler || DEFAULT_HANDLER };
}

/**
 * Resolve an `entry_file` against the node directory.
 *
 * `escapes` is true when the resolved artifact leaves the functions workspace
 * root — the case the server refuses with a Validation error, and the one the
 * CLI must catch before an upload rather than after.
 */
export function resolveEntryFile(
  nodeDir: string,
  entryFile: string,
  root: string
): { artifactPath: string; handler: string; escapes: boolean } {
  const { asset, handler } = splitEntryFile(entryFile);
  const artifactPath = path.resolve(nodeDir, asset);
  const rel = path.relative(path.resolve(root), artifactPath);
  const escapes = rel.startsWith('..') || path.isAbsolute(rel) || asset.startsWith('/');
  return { artifactPath, handler, escapes };
}

/** Every `raisin:Function` node declared under the package's functions tree. */
export function discoverFunctionNodes(packageRoot: string): FunctionNode[] {
  const root = functionsRoot(packageRoot);
  if (!fs.existsSync(root)) return [];
  const nodes: FunctionNode[] = [];
  for (const file of findFiles(root, '.node.yaml').sort()) {
    let doc: { node_type?: string; properties?: Record<string, unknown> };
    try {
      doc = yaml.parse(fs.readFileSync(file, 'utf-8')) || {};
    } catch {
      continue;
    }
    if (doc.node_type !== 'raisin:Function') continue;
    const props = doc.properties || {};
    const dir = path.dirname(file);
    const entryFile = typeof props.entry_file === 'string' ? props.entry_file : '';
    const resolved = entryFile ? resolveEntryFile(dir, entryFile, root) : null;
    nodes.push({
      file,
      dir,
      name: typeof props.name === 'string' ? props.name : path.basename(dir),
      language: typeof props.language === 'string' ? props.language : '',
      entryFile,
      artifactPath: resolved ? resolved.artifactPath : null,
      handler: resolved ? resolved.handler : DEFAULT_HANDLER,
      escapes: resolved ? resolved.escapes : false,
    });
  }
  return nodes;
}

/** Function nodes whose `entry_file` resolves to `artifactPath`. */
export function nodesForArtifact(nodes: FunctionNode[], artifactPath: string): FunctionNode[] {
  const target = path.resolve(artifactPath);
  return nodes.filter((n) => n.artifactPath === target);
}
