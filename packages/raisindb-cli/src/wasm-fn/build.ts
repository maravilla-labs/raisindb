/**
 * `raisindb function build` — run a guest toolchain and put its component where
 * the Function node's `entry_file` says it lives.
 *
 * The copy step is the whole point: the toolchain writes wherever it likes
 * (`target/wasm32-wasip2/release/…` for cargo), while the package ships the
 * artifact inside the Function node's directory under `content/functions/`.
 * Everything else — `sync --watch` pushing
 * the artifact as an asset, `deploy --install` packing it — already works once
 * the bytes are at that path.
 */

import crypto from 'crypto';
import fs from 'fs';
import path from 'path';
import { discoverFunctionNodes, findPackageRoot, nodesForArtifact } from './discover.js';
import { runCommand } from './toolchains.js';
import { MAX_ARTIFACT_BYTES, type FunctionNode, type WasmProject } from './types.js';

/** What one project build produced. */
export interface BuildResult {
  project: WasmProject;
  /** False when the toolchain failed or wrote nothing. */
  ok: boolean;
  /** Failure explanation, set when `ok` is false. */
  error?: string;
  /** Artifact size in bytes. */
  bytes?: number;
  /** Hex sha256 of the artifact. */
  sha256?: string;
  /** Wall time of the toolchain command. */
  durationMs?: number;
  /** Function nodes whose `entry_file` resolves to this artifact. */
  backs: FunctionNode[];
}

/** Human-readable byte size, matching the server's log style. */
export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ['KiB', 'MiB', 'GiB'];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(value < 10 ? 2 : 1)} ${units[unit]}`;
}

/** Build one project and copy its component into the Function node directory. */
export async function buildProject(
  project: WasmProject,
  nodes: FunctionNode[]
): Promise<BuildResult> {
  const backs = nodesForArtifact(nodes, project.artifactPath);
  const run = await runCommand(project.command, project.dir);
  if (run.code !== 0) {
    return { project, ok: false, error: `build command exited with ${run.code}`, backs };
  }
  if (!fs.existsSync(project.outputPath)) {
    return {
      project,
      ok: false,
      error:
        `build succeeded but ${project.outputPath} does not exist — ` +
        `set \`output:\` in ${path.basename(project.buildFile)} to the path the toolchain writes`,
      backs,
    };
  }

  const bytes = fs.readFileSync(project.outputPath);
  fs.mkdirSync(path.dirname(project.artifactPath), { recursive: true });
  fs.writeFileSync(project.artifactPath, bytes);

  return {
    project,
    ok: true,
    bytes: bytes.length,
    sha256: crypto.createHash('sha256').update(bytes).digest('hex'),
    durationMs: run.durationMs,
    backs,
  };
}

/** Print one build result the way the command reports it. */
export function printBuildResult(result: BuildResult, cwd = process.cwd()): void {
  const rel = (p: string) => path.relative(cwd, p) || p;
  if (!result.ok) {
    console.error(`x ${rel(result.project.dir)}: ${result.error}`);
    return;
  }
  console.log(`+ ${rel(result.project.dir)} -> ${rel(result.project.artifactPath)}`);
  console.log(
    `    ${formatBytes(result.bytes || 0)}  sha256 ${result.sha256}  (${result.durationMs} ms)`
  );
  if ((result.bytes || 0) > MAX_ARTIFACT_BYTES) {
    console.error(
      `    WARN artifact exceeds the ${formatBytes(MAX_ARTIFACT_BYTES)} server cap — upload will be refused`
    );
  }
  if (result.backs.length === 0) {
    console.log('    backs no Function node (nothing has an entry_file pointing here yet)');
    return;
  }
  console.log(`    backs ${result.backs.length} Function node(s):`);
  for (const node of result.backs) {
    console.log(`      ${node.name}  handler "${node.handler}"  (${rel(node.file)})`);
  }
}

/** Build every project under `target`; returns the process exit code. */
export async function buildProjects(
  projects: WasmProject[],
  packageRoot: string | null
): Promise<number> {
  const nodes = packageRoot ? discoverFunctionNodes(packageRoot) : [];
  let failed = 0;
  for (const project of projects) {
    const result = await buildProject(project, nodes);
    printBuildResult(result);
    if (!result.ok) failed += 1;
  }
  return failed === 0 ? 0 : 1;
}

/** Package root for a project, so a build can name the nodes it backs. */
export function packageRootFor(project: WasmProject): string | null {
  return findPackageRoot(project.dir) || findPackageRoot(project.nodeDir);
}

/**
 * Rebuild on change until interrupted.
 *
 * Watches only the project source, never the artifact it writes — a watcher
 * that saw its own output would rebuild forever. Pushing the new bytes to a
 * server is `raisindb sync --watch`'s job, running alongside.
 */
export async function watchProjects(projects: WasmProject[], packageRoot: string | null): Promise<void> {
  const chokidar = await import('chokidar');
  await buildProjects(projects, packageRoot);
  console.log('\nWatching for changes (Ctrl-C to stop)…');

  let building = false;
  let pending = false;
  const rebuild = async () => {
    if (building) {
      pending = true;
      return;
    }
    building = true;
    try {
      await buildProjects(projects, packageRoot);
    } finally {
      building = false;
      if (pending) {
        pending = false;
        await rebuild();
      }
    }
  };

  const watcher = chokidar.watch(
    projects.map((p) => p.dir),
    {
      ignoreInitial: true,
      ignored: /(^|[\\/])(node_modules|target|dist|build|\.git)([\\/]|$)|\.wasm$/,
      awaitWriteFinish: { stabilityThreshold: 200, pollInterval: 50 },
    }
  );
  watcher.on('all', () => {
    void rebuild();
  });
  await new Promise(() => {
    /* until interrupted */
  });
}
