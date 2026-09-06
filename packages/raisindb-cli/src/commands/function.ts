/**
 * `raisindb function build` and `raisindb function doctor`.
 *
 * Offline commands: they run a guest toolchain and read files, and never talk
 * to a server. Exit codes match `flow doctor` — 0 clean, 1 problems found,
 * 2 nothing to look at.
 */

import path from 'path';
import { buildProjects, watchProjects } from '../wasm-fn/build.js';
import { discoverProjects, findPackageRoot } from '../wasm-fn/discover.js';
import { runWasmDoctor, type DoctorReport, type Finding, type Severity } from '../wasm-fn/doctor.js';

/** Options for `raisindb function build`. */
export interface FunctionBuildOptions {
  /** Build every project under the package root rather than one. */
  all?: boolean;
  /** Rebuild on change until interrupted. */
  watch?: boolean;
  /** Build with the release profile (the default). */
  release?: boolean;
  /** Build with the debug profile — faster, much larger artifacts. */
  debug?: boolean;
}

/** Options for `raisindb function doctor`. */
export interface FunctionDoctorOptions {
  json?: boolean;
  /** Treat warnings as failures. */
  strict?: boolean;
}

const SEVERITY_TAG: Record<Severity, string> = {
  error: 'ERROR',
  warning: 'WARN ',
  hint: 'INFO ',
};

const SEVERITY_RANK: Record<Severity, number> = { error: 0, warning: 1, hint: 2 };

/**
 * Build the wasm function project(s) under `target`.
 *
 * `--all` widens the search to the whole package; without it the target must
 * be, or contain, exactly one project so a stray `raisindb function build` in a
 * package root does not spend minutes on every language at once.
 */
export async function functionBuild(
  target: string | undefined,
  options: FunctionBuildOptions = {}
): Promise<number> {
  const release = options.debug !== true;
  const start = path.resolve(target || process.cwd());
  const packageRoot = findPackageRoot(start);
  const scope = options.all ? packageRoot || start : start;
  const { projects, failures } = discoverProjects(scope, release);

  for (const failure of failures) {
    console.error(`x ${failure.file}: ${failure.error}`);
  }
  if (projects.length === 0) {
    if (failures.length > 0) return 1;
    console.error(
      `No raisin.build.yaml found under ${scope}.\n` +
        'Create one with `raisindb create function <name> --lang rust|go|ts`.'
    );
    return 2;
  }
  if (projects.length > 1 && !options.all) {
    console.error(`Found ${projects.length} wasm projects under ${scope}:`);
    for (const project of projects) console.error(`  ${path.relative(scope, project.dir)}`);
    console.error('Name one, or pass --all to build them all.');
    return 2;
  }

  if (options.watch) {
    await watchProjects(projects, packageRoot);
    return 0;
  }
  const code = await buildProjects(projects, packageRoot);
  return failures.length > 0 ? 1 : code;
}

function printFinding(finding: Finding): void {
  console.log(`    [${SEVERITY_TAG[finding.severity]}] ${finding.code} @${finding.where}: ${finding.message}`);
}

function printReport(report: DoctorReport): void {
  if (report.projects.length === 0 && report.findings.length === 0) {
    console.log(`No wasm function projects or nodes found under ${report.target}.`);
    console.log('Looked for raisin.build.yaml files and raisin:Function nodes with `language: wasm`.');
    return;
  }
  console.log(`Package: ${report.packageRoot || '(none — not inside a package)'}`);
  console.log(`Projects: ${report.projects.length}`);
  for (const project of report.projects) {
    console.log(`  ${path.relative(report.packageRoot || report.target, project.dir)} (${project.spec.lang})`);
  }
  console.log('');
  const sorted = [...report.findings].sort(
    (a, b) => SEVERITY_RANK[a.severity] - SEVERITY_RANK[b.severity] || a.where.localeCompare(b.where)
  );
  for (const finding of sorted) printFinding(finding);

  const errors = report.findings.filter((f) => f.severity === 'error').length;
  const warnings = report.findings.filter((f) => f.severity === 'warning').length;
  console.log('');
  console.log(`Summary: ${errors} error(s), ${warnings} warning(s)`);
}

/** Run the wasm function doctor; returns the process exit code. */
export function functionDoctor(
  target: string | undefined,
  options: FunctionDoctorOptions = {}
): number {
  const report = runWasmDoctor(target || process.cwd(), { strict: options.strict });
  if (options.json) {
    console.log(
      JSON.stringify(
        {
          target: report.target,
          packageRoot: report.packageRoot,
          projects: report.projects.map((p) => ({
            dir: p.dir,
            lang: p.spec.lang,
            artifact: p.artifactPath,
            command: p.command,
          })),
          findings: report.findings,
          exitCode: report.exitCode,
        },
        null,
        2
      )
    );
  } else {
    printReport(report);
  }
  return report.exitCode;
}
