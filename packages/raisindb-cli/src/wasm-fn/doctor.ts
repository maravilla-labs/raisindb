/**
 * `raisindb function doctor` — everything about a wasm function that can be
 * checked without a server.
 *
 * The checks worth having are the ones whose absence is a runtime-only failure
 * on a deployed server:
 *
 * - an `entry_file` naming a handler the project never registers (the host must
 *   not keep an allow-list, so nothing catches this until invocation);
 * - a parent-relative `entry_file` that escapes the functions workspace (the
 *   server refuses it at load with a Validation error);
 * - an artifact that is missing, or bigger than the server's cap.
 */

import fs from 'fs';
import path from 'path';
import { discoverFunctionNodes, discoverProjects, findPackageRoot, functionsRoot } from './discover.js';
import { registeredHandlers } from './handlers.js';
import { jcoWit, toolchainFor } from './toolchains.js';
import { formatBytes } from './build.js';
import { MAX_ARTIFACT_BYTES, type FunctionNode, type WasmProject } from './types.js';

/** How badly a finding matters. */
export type Severity = 'error' | 'warning' | 'hint';

/** One thing the doctor noticed. */
export interface Finding {
  severity: Severity;
  /** Stable code, e.g. `WASM_HANDLER_NOT_REGISTERED`. */
  code: string;
  /** What the finding is about — a node name or a project directory. */
  where: string;
  message: string;
}

/** The whole report. */
export interface DoctorReport {
  target: string;
  packageRoot: string | null;
  projects: WasmProject[];
  nodes: FunctionNode[];
  findings: Finding[];
  /** 0 = clean, 1 = errors (or warnings under `--strict`), 2 = nothing found. */
  exitCode: number;
}

const err = (code: string, where: string, message: string): Finding => ({
  severity: 'error',
  code,
  where,
  message,
});
const warn = (code: string, where: string, message: string): Finding => ({
  severity: 'warning',
  code,
  where,
  message,
});
const hint = (code: string, where: string, message: string): Finding => ({
  severity: 'hint',
  code,
  where,
  message,
});

/** Toolchain availability for every language present in the report. */
function checkToolchains(projects: WasmProject[]): Finding[] {
  const findings: Finding[] = [];
  for (const lang of new Set(projects.map((p) => p.spec.lang))) {
    for (const tool of toolchainFor(lang)) {
      if (tool.version) {
        findings.push(hint('WASM_TOOLCHAIN', lang, `${tool.name}: ${tool.version}`));
      } else if (tool.required) {
        findings.push(
          err('WASM_TOOLCHAIN_MISSING', lang, `${tool.name} is not installed — cannot build ${lang} functions`)
        );
      }
    }
  }
  return findings;
}

/** Build-file ↔ node-directory consistency, and the artifact itself. */
function checkProject(project: WasmProject, cwd: string, toolchains: boolean): Finding[] {
  const findings: Finding[] = [];
  const where = path.relative(cwd, project.dir) || project.dir;

  if (!fs.existsSync(project.nodeDir)) {
    findings.push(
      err('WASM_NODE_DIR_MISSING', where, `node_dir does not exist: ${project.nodeDir}`)
    );
    return findings;
  }
  const nodeYaml = path.join(project.nodeDir, '.node.yaml');
  if (!fs.existsSync(nodeYaml)) {
    findings.push(
      warn('WASM_NODE_YAML_MISSING', where, `no .node.yaml in ${project.nodeDir} — nothing will ship this artifact`)
    );
  }

  if (!fs.existsSync(project.artifactPath)) {
    findings.push(
      warn('WASM_ARTIFACT_MISSING', where, `artifact not built yet — run \`raisindb function build\``)
    );
    return findings;
  }

  const size = fs.statSync(project.artifactPath).size;
  if (size > MAX_ARTIFACT_BYTES) {
    findings.push(
      err(
        'WASM_ARTIFACT_TOO_LARGE',
        where,
        `artifact is ${formatBytes(size)}, over the ${formatBytes(MAX_ARTIFACT_BYTES)} server cap`
      )
    );
  } else {
    findings.push(hint('WASM_ARTIFACT', where, `${formatBytes(size)} at ${project.artifactPath}`));
  }

  if (!toolchains) return findings;
  const wit = jcoWit(project.artifactPath);
  if (wit === null) {
    findings.push(hint('WASM_WORLD_SKIPPED', where, 'world check skipped (jco not installed)'));
  } else {
    if (!/raisin:function\/host/.test(wit)) {
      findings.push(err('WASM_WORLD_IMPORT', where, 'component does not import raisin:function/host'));
    }
    if (!/\bhandler\b/.test(wit)) {
      findings.push(err('WASM_WORLD_EXPORT', where, 'component does not export `handler`'));
    }
  }
  return findings;
}

/** Everything about one Function node's `entry_file`. */
function checkNode(node: FunctionNode, projects: WasmProject[], root: string): Finding[] {
  const findings: Finding[] = [];
  if (node.language !== 'wasm') return findings;

  if (!node.entryFile) {
    findings.push(err('WASM_ENTRY_FILE_MISSING', node.name, 'language is wasm but entry_file is not set'));
    return findings;
  }
  if (node.escapes) {
    findings.push(
      err(
        'WASM_ENTRY_FILE_ESCAPES',
        node.name,
        `entry_file "${node.entryFile}" resolves outside the functions workspace (${root}) — the server refuses it`
      )
    );
    return findings;
  }
  const artifact = node.artifactPath as string;
  if (!fs.existsSync(artifact)) {
    findings.push(
      warn('WASM_ARTIFACT_MISSING', node.name, `entry_file target does not exist: ${artifact}`)
    );
  } else if (fs.statSync(artifact).size > MAX_ARTIFACT_BYTES) {
    findings.push(
      err(
        'WASM_ARTIFACT_TOO_LARGE',
        node.name,
        `entry_file target is over the ${formatBytes(MAX_ARTIFACT_BYTES)} server cap`
      )
    );
  }

  const producer = projects.find((p) => p.artifactPath === artifact);
  if (!producer) {
    findings.push(
      warn(
        'WASM_NO_PRODUCER',
        node.name,
        `no raisin.build.yaml builds ${artifact} — the handler name cannot be checked`
      )
    );
    return findings;
  }
  const scan = registeredHandlers(producer);
  if (scan.note) {
    findings.push(hint('WASM_HANDLERS_UNKNOWN', node.name, `handler names not checked: ${scan.note}`));
  } else if (!scan.names.includes(node.handler)) {
    findings.push(
      err(
        'WASM_HANDLER_NOT_REGISTERED',
        node.name,
        `entry_file selects handler "${node.handler}", but ${path.basename(producer.dir)} registers: ${scan.names.join(', ')}`
      )
    );
  } else {
    findings.push(hint('WASM_HANDLER', node.name, `handler "${node.handler}" is registered`));
  }
  return findings;
}

/** Run every check under `target`. */
export function runWasmDoctor(
  target: string,
  options: { strict?: boolean; toolchains?: boolean } = {}
): DoctorReport {
  // Probing the machine (cargo, TinyGo, jco) is what the command is for, and
  // what a unit test must not depend on — hence the switch, not two code paths.
  const toolchains = options.toolchains !== false;
  const resolved = path.resolve(target);
  const packageRoot = findPackageRoot(resolved);
  // Findings are located relative to the package, so a report reads the same
  // wherever the command was run from.
  const cwd = packageRoot || process.cwd();
  const { projects, failures } = discoverProjects(resolved);
  const nodes = packageRoot ? discoverFunctionNodes(packageRoot) : [];

  const findings: Finding[] = failures.map((f) =>
    err('WASM_BUILD_FILE_INVALID', path.relative(cwd, f.file) || f.file, f.error)
  );
  if (toolchains) findings.push(...checkToolchains(projects));
  for (const project of projects) findings.push(...checkProject(project, cwd, toolchains));

  // Every wasm node in the package is checked, even when `target` named ONE
  // project: a node whose entry_file escapes the workspace, or points at an
  // artifact no project builds, is exactly the case a per-project scope would
  // hide — and it is the case that fails at runtime on a deployed server.
  const root = packageRoot ? functionsRoot(packageRoot) : resolved;
  const wasmNodes = nodes.filter((n) => n.language === 'wasm');
  for (const node of wasmNodes) findings.push(...checkNode(node, projects, root));

  const errors = findings.filter((f) => f.severity === 'error').length;
  const warnings = findings.filter((f) => f.severity === 'warning').length;
  let exitCode = 0;
  if (projects.length === 0 && wasmNodes.length === 0 && failures.length === 0) exitCode = 2;
  else if (errors > 0 || (options.strict && warnings > 0)) exitCode = 1;

  return { target: resolved, packageRoot, projects, nodes, findings, exitCode };
}
