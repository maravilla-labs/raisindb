/**
 * Flow doctor: static analysis of designer-format flow definitions.
 */

import { checkFlow } from './checks.js';
import { classifyDefinition, discoverFlows } from './load.js';
import { PackageContext } from './package-context.js';
import type { Finding, FlowReport, ParseFailure, Severity } from './types.js';

export interface DoctorResult {
  target: string;
  reports: FlowReport[];
  failures: ParseFailure[];
  summary: {
    flows: number;
    analyzed: number;
    errors: number;
    warnings: number;
    suggestions: number;
    parseFailures: number;
  };
  exitCode: number; // 0 clean, 1 errors (or warnings with --strict), 2 parse failure
}

export interface DoctorOptions {
  strict?: boolean;
}

export interface WorkflowDoctorOptions {
  /** File the definition came from (used in messages only). */
  filePath?: string;
  /** Package context for function-reference resolution (optional). */
  pkg?: PackageContext | null;
}

export interface WorkflowDoctorResult {
  /** How the definition was classified ('invalid' = not a flow definition). */
  format: 'designer' | 'runtime' | 'invalid';
  findings: Finding[];
}

/**
 * Pure entry point: run every doctor check against a single parsed
 * workflow_data value (designer format). Shared by `raisindb flow doctor`
 * and package validation (`raisindb package validate` / create / deploy).
 *
 * Runtime-format definitions are skipped (no findings); values that are not
 * a recognizable flow definition produce a single INVALID_WORKFLOW_DATA error.
 */
export function doctorWorkflowData(
  workflowData: unknown,
  options: WorkflowDoctorOptions = {}
): WorkflowDoctorResult {
  const filePath = options.filePath ?? '<inline>';
  const source = classifyDefinition(filePath, workflowData, filePath);

  if (!source) {
    return {
      format: 'invalid',
      findings: [
        {
          code: 'INVALID_WORKFLOW_DATA',
          severity: 'error',
          nodeId: '',
          field: 'workflow_data',
          message:
            'workflow_data is not a recognizable flow definition (expected designer format with a top-level nodes array).',
        },
      ],
    };
  }

  if (source.format !== 'designer' || !source.definition) {
    // Runtime format: not analyzed by the doctor (same as `flow doctor`).
    return { format: 'runtime', findings: [] };
  }

  return { format: 'designer', findings: checkFlow(source.definition, options.pkg ?? null) };
}

export function runDoctor(target: string, options: DoctorOptions = {}): DoctorResult {
  const { sources, failures, packageDir } = discoverFlows(target);
  const pkg = packageDir ? new PackageContext(packageDir) : null;

  const reports: FlowReport[] = sources.map((source) => ({
    source,
    findings: source.format === 'designer' && source.definition ? checkFlow(source.definition, pkg) : [],
  }));

  const count = (severity: Severity) =>
    reports.reduce((n, r) => n + r.findings.filter((f) => f.severity === severity).length, 0);

  const summary = {
    flows: sources.length,
    analyzed: sources.filter((s) => s.format === 'designer').length,
    errors: count('error'),
    warnings: count('warning'),
    suggestions: count('suggestion'),
    parseFailures: failures.length,
  };

  let exitCode = 0;
  if (summary.errors > 0) exitCode = 1;
  if (options.strict && summary.warnings > 0) exitCode = Math.max(exitCode, 1);
  if (failures.length > 0) exitCode = 2;

  return { target, reports, failures, summary, exitCode };
}
