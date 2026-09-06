/**
 * Deciding WHAT `raisindb function run|test --server` runs, and HOW.
 *
 * Two separable questions, both pure enough to unit-test without a server:
 *
 * 1. `resolveRunTarget` — which Function node, which artifact, which handler.
 *    A path may name a toolchain project (`wasm/<ns>/<name>`), a Function node
 *    directory (`content/functions/lib/<ns>/<name>`), or a package root with
 *    exactly one wasm function in it.
 * 2. `planRun` — invoke the deployed function, or upload the local artifact and
 *    run it directly. The rule is deliberately conservative: anything short of
 *    "the server is provably holding these exact bytes" uploads again, because
 *    a dev loop that silently runs stale code is worse than one extra upload.
 */

import fs from 'fs';
import path from 'path';
import {
  contentBase,
  discoverFunctionNodes,
  discoverProjects,
  findPackageRoot,
  nodesForArtifact,
} from './discover.js';
import { DEFAULT_HANDLER, type FunctionNode, type WasmProject } from './types.js';

/** Everything `function run` needs from the local package. */
export interface RunTarget {
  /** Package root (the directory holding `manifest.yaml`). */
  packageRoot: string;
  /** The Function node being run. */
  node: FunctionNode;
  /** Absolute path of the artifact its `entry_file` resolves to. */
  artifactPath: string;
  /** Handler to call — the node's, unless `--handler` overrode it. */
  handler: string;
  /** True when `--handler` selected something other than the node's own. */
  handlerOverridden: boolean;
  /** The toolchain project that builds the artifact, when one was found. */
  project?: WasmProject;
}

/** Options that steer target resolution. */
export interface ResolveTargetOptions {
  /** `--handler`: call this handler instead of the node's `entry_file` one. */
  handler?: string;
}

/** A workspace-relative location: the workspace name and the node path in it. */
export interface WorkspaceLocation {
  workspace: string;
  /** Node path WITHOUT a leading slash, as the repository URL wants it. */
  nodePath: string;
}

/**
 * Where an artifact lives on the server, derived from where it lives on disk.
 *
 * `content/functions/lib/demo/greet/main.wasm` is workspace `functions`, node
 * path `lib/demo/greet/main.wasm`. A package without a `content/` directory
 * uses the root the same way — `contentBase` already answers which it is.
 */
export function workspaceLocation(packageRoot: string, artifactPath: string): WorkspaceLocation {
  const rel = path.relative(contentBase(packageRoot), path.resolve(artifactPath));
  if (rel.startsWith('..') || path.isAbsolute(rel)) {
    throw new Error(
      `${artifactPath} is outside the package content tree (${contentBase(packageRoot)})`
    );
  }
  const segments = rel.split(path.sep);
  if (segments.length < 2) {
    throw new Error(`${artifactPath} is not inside a workspace directory`);
  }
  return { workspace: segments[0], nodePath: segments.slice(1).join('/') };
}

/**
 * True when `function run` can drive this node.
 *
 * WebAssembly, JavaScript and Starlark all qualify: the deploy step is the
 * same multipart upload of whatever file `entry_file` names, and the server's
 * code loader reads a text asset exactly as it reads an artifact. Only the
 * BUILD differs between them, and `run` does not build.
 *
 * SQL functions are excluded: their code is not a file beside the node.
 */
function isRunnableNode(node: FunctionNode): boolean {
  return node.language === 'wasm' || node.language === 'javascript' || node.language === 'starlark';
}

/** Nodes the target directory selects, before `--handler` narrows them. */
function candidatesFor(start: string, packageRoot: string): {
  nodes: FunctionNode[];
  project?: WasmProject;
} {
  const all = discoverFunctionNodes(packageRoot).filter(isRunnableNode);
  const resolved = path.resolve(start);

  // A Function node directory (or its .node.yaml) selects exactly that node.
  const direct = all.filter((n) => n.dir === resolved || n.file === resolved);
  if (direct.length > 0) return { nodes: direct };

  // A toolchain project selects every node backed by its artifact.
  const { projects } = discoverProjects(resolved);
  if (projects.length === 1) {
    const project = projects[0];
    return { nodes: nodesForArtifact(all, project.artifactPath), project };
  }
  if (projects.length > 1) {
    // A directory above several projects: fall through to the package-wide set
    // so the error names the Function nodes rather than the build files.
    return { nodes: all };
  }

  // Anything else (the package root, typically) offers the whole package.
  return { nodes: all };
}

/** Format a candidate list for an ambiguity error. */
function describe(nodes: FunctionNode[], packageRoot: string): string {
  return nodes
    .map((n) => `  ${n.name} (${path.relative(packageRoot, n.dir)}) -> ${n.entryFile || '(unset)'}`)
    .join('\n');
}

/**
 * Resolve the Function node, artifact and handler a run targets.
 *
 * Throws with a listing rather than guessing when the target is ambiguous —
 * running the wrong function against a live server is not a recoverable typo.
 */
export function resolveRunTarget(
  target: string | undefined,
  options: ResolveTargetOptions = {}
): RunTarget {
  const start = path.resolve(target || process.cwd());
  if (!fs.existsSync(start)) throw new Error(`No such path: ${start}`);
  const packageRoot = findPackageRoot(start);
  if (!packageRoot) {
    throw new Error(
      `${start} is not inside a package — no manifest.yaml found above it. ` +
        'Run this from a package created by `raisindb create function`.'
    );
  }

  const { nodes, project } = candidatesFor(start, packageRoot);
  if (nodes.length === 0) {
    throw new Error(
      `No runnable Function node found under ${start}.\n` +
        'Create one with `raisindb create function <name> --lang rust|go|js|starlark`.'
    );
  }

  const wanted = options.handler?.trim();
  let node: FunctionNode;
  if (wanted) {
    const match = nodes.find((n) => n.handler === wanted);
    // No node declares this handler: still legal — the guest owns its handler
    // namespace, so run the artifact and let it answer. The node only supplies
    // the artifact path in that case.
    node = match || nodes[0];
  } else if (nodes.length === 1) {
    node = nodes[0];
  } else {
    throw new Error(
      `${nodes.length} wasm functions under ${start}:\n${describe(nodes, packageRoot)}\n` +
        'Name one, or select a handler with --handler.'
    );
  }

  if (!node.artifactPath) {
    throw new Error(`${node.name}: its .node.yaml has no entry_file, so no artifact to run.`);
  }
  if (node.escapes) {
    throw new Error(
      `${node.name}: entry_file '${node.entryFile}' resolves outside the functions workspace — ` +
        'the server refuses this too.'
    );
  }

  const handler = wanted || node.handler || DEFAULT_HANDLER;
  return {
    packageRoot,
    node,
    artifactPath: node.artifactPath,
    handler,
    handlerOverridden: !!wanted && wanted !== node.handler,
    project,
  };
}

/** How a run reaches the server. */
export type RunMode = 'invoke' | 'run-file';

/** The chosen route plus the one-line reason the CLI prints. */
export interface RunPlan {
  mode: RunMode;
  reason: string;
}

/** Inputs `planRun` decides from. */
export interface RunPlanInput {
  /** `--handler` asked for something other than the node's own handler. */
  handlerOverridden: boolean;
  /** The `raisin:Function` node exists on the server. */
  functionExists: boolean;
  /** Hex sha256 the server recorded for the artifact, when it recorded one. */
  serverHash: string | null;
  /** Hex sha256 of the local artifact. */
  localHash: string;
}

/**
 * Invoke the deployed function, or upload and run the local artifact?
 *
 * Only a recorded server hash that matches the local bytes takes the invoke
 * path. Note that a CLI multipart upload records NO hash (only the package
 * installer writes `content_hash`), so a `function run` loop re-uploads every
 * time until the package is deployed — which is the honest answer: nothing on
 * the server can prove the bytes otherwise.
 */
export function planRun(input: RunPlanInput): RunPlan {
  if (input.handlerOverridden) {
    return { mode: 'run-file', reason: 'a --handler override runs the artifact directly' };
  }
  if (!input.functionExists) {
    return { mode: 'run-file', reason: 'the function is not on the server yet' };
  }
  if (!input.serverHash) {
    return { mode: 'run-file', reason: 'the server artifact records no content hash' };
  }
  if (input.serverHash !== input.localHash) {
    return { mode: 'run-file', reason: 'the local artifact differs from the server copy' };
  }
  return { mode: 'invoke', reason: 'the server already holds these exact bytes' };
}

/** Read `--input` / `--input-file` into the JSON value a run sends. */
export function resolveInput(
  inline: string | undefined,
  file: string | undefined
): unknown {
  if (inline && file) {
    throw new Error('--input and --input-file are mutually exclusive');
  }
  const raw = file ? fs.readFileSync(path.resolve(file), 'utf-8') : inline;
  if (raw === undefined || raw.trim() === '') return {};
  try {
    return JSON.parse(raw) as unknown;
  } catch (error) {
    const where = file ? path.resolve(file) : '--input';
    throw new Error(`${where}: not valid JSON — ${error instanceof Error ? error.message : error}`);
  }
}
