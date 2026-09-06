/**
 * `raisindb create function <name> --lang rust|go|ts [--handler h] [--into project]`
 *
 * Two shapes, one command:
 *
 * - **New project.** Scaffolds a toolchain project under `wasm/<ns>/<name>/`
 *   and a Function node under `content/functions/lib/<ns>/<name>/`, with the
 *   artifact path they agree on. Source stays outside `content/` because
 *   `sync` maps every non-YAML file under it to a node.
 * - **`--into <project>`.** Adds a SECOND named handler to an existing wasm
 *   project and writes a Function node whose `entry_file` points at that
 *   project's artifact (`../<other>/main.wasm:<handler>`). No second toolchain
 *   project, no second component — one artifact, N functions, which is what
 *   makes a package of TypeScript functions ship 12 MiB instead of 200.
 */

import fs from 'fs';
import path from 'path';
import yaml from 'yaml';
import { writeFileTree } from '../templates/render.js';
import {
  entryFileFor,
  nodeYaml,
  titleCase,
  wasmFunctionFiles,
  type SdkRef,
  type WasmFnVars,
} from '../templates/wasm/index.js';
import { addHandler } from '../wasm-fn/add-handler.js';
import { contentBase, discoverProjects, findPackageRoot, functionsRoot, loadProject } from '../wasm-fn/discover.js';
import { sourceFunctionFiles } from '../templates/source/index.js';
import {
  isSourceLang,
  SOURCE_ENTRY_FILE,
  SOURCE_LANGS,
  SOURCE_NODE_LANGUAGE,
  WASM_LANGS,
  type FunctionLang,
  type WasmLang,
  type WasmProject,
} from '../wasm-fn/types.js';

/**
 * Release tag scaffolded projects pin their guest SDK to.
 *
 * Bump this with each release that changes the SDK or the WIT contract; a
 * scaffold pinned to a tag keeps building after the SDK moves on.
 */
const SDK_GIT_TAG = 'v0.5.0';

/** Options for `raisindb create function`. */
export interface CreateFunctionOptions {
  /** Guest language. Required unless `--into` supplies it. */
  lang?: string;
  /** Namespace segment under `content/functions/lib/`. */
  ns?: string;
  /** Package directory. Defaults to the nearest package root above cwd. */
  dir?: string;
  /** Handler name. Defaults to `default` for a new project, `<name>` for `--into`. */
  handler?: string;
  /** An existing wasm project to add this function's handler to. */
  into?: string;
  /** One-line description for the node and the scaffold. */
  description?: string;
}

const SLUG_RE = /^[a-z0-9]+(?:-[a-z0-9]+)*$/;
const HANDLER_RE = /^[A-Za-z0-9][A-Za-z0-9_-]*$/;

/** Package name from a manifest, for the default namespace. */
function manifestNamespace(packageRoot: string): string {
  for (const name of ['manifest.yaml', 'manifest.yml']) {
    const file = path.join(packageRoot, name);
    if (!fs.existsSync(file)) continue;
    try {
      const parsed = yaml.parse(fs.readFileSync(file, 'utf-8')) as { name?: string };
      if (parsed?.name && SLUG_RE.test(parsed.name)) return parsed.name;
    } catch {
      /* fall through to the default */
    }
  }
  return 'app';
}

/**
 * How the scaffold reaches the guest SDK.
 *
 * Inside a checkout of this monorepo the SDKs are not published yet, so a path
 * dependency is the only thing that builds; anywhere else it is a version.
 */
function sdkRef(lang: WasmLang, projectDir: string, packageRoot: string): SdkRef {
  const relative = { rust: 'sdks/rust/raisin-sdk', go: 'sdks/go/raisin', ts: 'sdks/ts/function-wasm' }[lang];
  let dir = packageRoot;
  for (;;) {
    const candidate = path.join(dir, relative);
    if (fs.existsSync(candidate)) {
      const rel = path.relative(projectDir, candidate).split(path.sep).join('/');
      return { kind: 'path', value: rel.startsWith('.') ? rel : `./${rel}` };
    }
    const parent = path.dirname(dir);
    if (parent === dir) break;
    dir = parent;
  }
  // Outside the monorepo the SDKs come from the public repo, pinned to a
  // release tag. NOT a registry version: the Rust crate is `publish = false`
  // and the Go SDK is a subdirectory module, so a bare version requirement
  // scaffolds a project that cannot resolve its dependency.
  return { kind: 'git', value: SDK_GIT_TAG };
}

/** Make sure `.rapignore` keeps toolchain source out of the package. */
function ensureRapignore(packageRoot: string): boolean {
  const file = path.join(packageRoot, '.rapignore');
  const line = 'wasm/';
  const banner = `# Toolchain projects are SOURCE, not package content: only the built
# \`main.wasm\` under content/functions/ ships. \`raisindb sync\` treats every
# non-yaml file it finds as an asset, so Cargo.toml / main.go must never be
# visible to it.
${line}
`;
  if (!fs.existsSync(file)) {
    fs.writeFileSync(file, banner);
    return true;
  }
  const existing = fs.readFileSync(file, 'utf-8');
  if (existing.split('\n').some((l) => l.trim() === line || l.trim() === 'wasm')) return false;
  fs.writeFileSync(file, `${existing.replace(/\s*$/, '\n')}\n${banner}`);
  return true;
}

/** Resolve `--into` to a toolchain project: a path, or a project/node name. */
function resolveInto(into: string, packageRoot: string): WasmProject {
  const direct = path.resolve(into);
  if (fs.existsSync(path.join(direct, 'raisin.build.yaml'))) {
    return loadProject(path.join(direct, 'raisin.build.yaml'));
  }
  const { projects } = discoverProjects(packageRoot);
  const match = projects.filter(
    (p) => path.basename(p.dir) === into || path.basename(p.nodeDir) === into
  );
  if (match.length === 1) return match[0];
  if (match.length > 1) {
    throw new Error(
      `--into "${into}" is ambiguous: ${match.map((p) => path.relative(packageRoot, p.dir)).join(', ')}`
    );
  }
  const known = projects.map((p) => path.basename(p.dir)).join(', ') || '(none)';
  throw new Error(`--into "${into}" is not a wasm project in ${packageRoot}. Known projects: ${known}`);
}

/** Scaffold a wasm function, or add a handler to an existing project. */
export async function createFunction(
  name: string,
  options: CreateFunctionOptions = {}
): Promise<void> {
  const slug = name.trim().toLowerCase();
  if (!SLUG_RE.test(slug)) {
    throw new Error(`Invalid function name "${name}". Use lower-kebab-case, e.g. "greet" or "on-order".`);
  }

  const base = path.resolve(options.dir || process.cwd());
  const packageRoot = findPackageRoot(base);
  if (!packageRoot) {
    throw new Error(
      `No manifest.yaml found at or above ${base}. Run this inside a package directory, or pass --dir.`
    );
  }

  const into = options.into ? resolveInto(options.into, packageRoot) : null;
  const lang = (options.lang || into?.spec.lang) as FunctionLang | undefined;
  const allLangs = [...WASM_LANGS, ...SOURCE_LANGS];
  if (!lang || !allLangs.includes(lang as never)) {
    throw new Error(`--lang is required and must be one of ${allLangs.join(', ')}.`);
  }
  if (into && isSourceLang(lang)) {
    throw new Error(
      `--into shares a compiled ARTIFACT between nodes; ${lang} functions ship source. ` +
        `Add another exported handler to the existing file and create a node whose ` +
        `entry_file names it.`
    );
  }
  if (into && options.lang && options.lang !== into.spec.lang) {
    throw new Error(
      `--into ${options.into} is a ${into.spec.lang} project; --lang ${options.lang} cannot share its artifact.`
    );
  }

  const handler = (options.handler || (into ? slug : 'default')).trim();
  if (!HANDLER_RE.test(handler)) {
    throw new Error(`Invalid handler name "${handler}". Use letters, digits, "-" or "_".`);
  }

  const ns = (options.ns || manifestNamespace(packageRoot)).toLowerCase();
  if (!SLUG_RE.test(ns)) throw new Error(`Invalid namespace "${ns}". Use lower-kebab-case.`);

  const nodeDir = path.join(contentBase(packageRoot), 'functions', 'lib', ns, slug);
  const nodePath = path.relative(packageRoot, nodeDir).split(path.sep).join('/');

  if (isSourceLang(lang)) {
    if (fs.existsSync(path.join(nodeDir, '.node.yaml'))) {
      throw new Error(`${nodeDir}/.node.yaml already exists. Choose a different name.`);
    }
    const sourceHandler = (options.handler || 'handler').trim();
    if (!HANDLER_RE.test(sourceHandler)) {
      throw new Error(`Invalid handler name "${sourceHandler}".`);
    }
    const files = sourceFunctionFiles(
      {
        name: slug,
        ns,
        lang,
        handler: sourceHandler,
        description:
          options.description || `RaisinDB function "${slug}".`,
      },
      nodePath
    );
    const written = writeFileTree(packageRoot, files);
    const entry = `${SOURCE_ENTRY_FILE[lang]}:${sourceHandler}`;

    console.log(`\nScaffolded ${SOURCE_NODE_LANGUAGE[lang]} function "${slug}" in ${packageRoot}\n`);
    console.log(`  Node:      ${nodePath}/.node.yaml   (entry_file: ${entry})`);
    console.log(`  Source:    ${nodePath}/${SOURCE_ENTRY_FILE[lang]}`);
    console.log(`  Files:     ${written}`);
    console.log(`\nNothing to build — the source ships as-is.\n`);
    console.log(`Next steps:`);
    console.log(`  1. raisindb deploy . --repo <repo> --install`);
    console.log(`  2. raisindb sync . --watch        # edit-and-push loop`);
    console.log(
      `\nNote: \`raisindb function run\` is WebAssembly-only today; invoke this one\n` +
        `over HTTP or from the admin console.`
    );
    return;
  }

  const projectPath = `wasm/${ns}/${slug}`;
  const projectDir = path.join(packageRoot, projectPath);

  if (fs.existsSync(path.join(nodeDir, '.node.yaml'))) {
    throw new Error(`${nodeDir}/.node.yaml already exists. Choose a different name.`);
  }
  if (!into && fs.existsSync(projectDir)) {
    throw new Error(`${projectDir} already exists. Choose a different name, or use --into ${slug}.`);
  }

  const vars: WasmFnVars = {
    name: slug,
    ns,
    handler,
    title: titleCase(slug),
    description: options.description || `RaisinDB function "${slug}", compiled to a WebAssembly component.`,
    lang,
    sdk: sdkRef(lang, projectDir, packageRoot),
    nodeDirRel: path.relative(projectDir, nodeDir).split(path.sep).join('/'),
  };

  if (into) {
    createIntoExisting(vars, into, packageRoot, nodeDir, nodePath);
    return;
  }

  const files = wasmFunctionFiles(vars, nodePath, projectPath);
  const count = writeFileTree(packageRoot, files);
  const rapignore = ensureRapignore(packageRoot);

  console.log(`\nScaffolded wasm function "${slug}" (${lang}) in ${packageRoot}\n`);
  console.log(`  Node:      ${nodePath}/.node.yaml   (entry_file: ${entryFileFor(handler)})`);
  console.log(`  Project:   ${projectPath}/`);
  console.log(`  Files:     ${count}${rapignore ? ' (+ .rapignore)' : ''}`);
  console.log(`\nNext steps:`);
  console.log(`  1. raisindb function doctor ${projectPath}`);
  console.log(`  2. raisindb function build ${projectPath}`);
  console.log(`  3. raisindb deploy . --repo <repo> --install`);
  console.log('');
}

/** The `--into` half: a node pointing at another project's artifact. */
function createIntoExisting(
  vars: WasmFnVars,
  into: WasmProject,
  packageRoot: string,
  nodeDir: string,
  nodePath: string
): void {
  const relArtifact = path.relative(nodeDir, into.artifactPath).split(path.sep).join('/');
  const entryFile = `${relArtifact}:${vars.handler}`;
  const resolved = path.resolve(nodeDir, relArtifact);
  const root = functionsRoot(packageRoot);
  if (path.relative(root, resolved).startsWith('..')) {
    throw new Error(
      `entry_file "${entryFile}" would resolve outside the functions workspace (${root}); ` +
        'the server refuses such a path. Put both nodes under content/functions/.'
    );
  }

  const result = addHandler(into, vars.handler);
  const doc = nodeYaml(vars).replace(
    /^  entry_file: .*$/m,
    `  entry_file: ${entryFile}`
  );
  writeFileTree(packageRoot, [{ path: `${nodePath}/.node.yaml`, content: doc }]);

  console.log(`\nAdded handler "${vars.handler}" to ${path.relative(packageRoot, into.dir)}\n`);
  console.log(`  Node:      ${nodePath}/.node.yaml`);
  console.log(`  entry_file: ${entryFile}   (the SAME artifact as ${path.basename(into.nodeDir)})`);
  for (const file of result.changed) console.log(`  Updated:   ${path.relative(packageRoot, file)}`);
  for (const warning of result.warnings) console.log(`  Note:      ${warning}`);
  console.log(`\nRebuild the shared artifact:  raisindb function build ${path.relative(packageRoot, into.dir)}`);
  console.log('');
}
