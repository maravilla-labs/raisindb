/**
 * `raisindb flow doctor <path>` and `raisindb flow explain <path>`
 *
 * Static analysis and debug helpers for raisin:Flow definitions
 * (designer format). Plain console output; exit codes:
 *   0 = clean, 1 = errors found (warnings too with --strict), 2 = parse failure
 */

import path from 'path';
import { runDoctor, type DoctorResult } from '../flow/doctor.js';
import { discoverFlows } from '../flow/load.js';
import { explainFlow } from '../flow/explain.js';
import type { Finding, Severity } from '../flow/types.js';

export interface FlowDoctorOptions {
  json?: boolean;
  strict?: boolean;
}

const SEVERITY_TAG: Record<Severity, string> = {
  error: 'ERROR',
  warning: 'WARN ',
  suggestion: 'HINT ',
};

const SEVERITY_RANK: Record<Severity, number> = { error: 0, warning: 1, suggestion: 2 };

function printFinding(finding: Finding): void {
  const where = finding.nodeId ? `@${finding.nodeId}` : '@flow';
  const field = finding.field ? ` (${finding.field})` : '';
  console.log(`    [${SEVERITY_TAG[finding.severity]}] ${finding.code} ${where}${field}: ${finding.message}`);
}

function printHumanReport(result: DoctorResult): void {
  if (result.reports.length === 0 && result.failures.length === 0) {
    console.log(`No flow definitions found under ${result.target}.`);
    console.log('Looked for raisin:Flow YAML nodes, designer-format YAML/JSON, and workflow_data literals in JS files.');
    return;
  }

  for (const report of result.reports) {
    const rel = path.relative(process.cwd(), report.source.file) || report.source.file;
    const note = report.source.note ? ` — ${report.source.note}` : '';
    if (report.source.format !== 'designer') {
      console.log(`- ${report.source.name} (${rel})${note}: skipped`);
      continue;
    }
    if (report.findings.length === 0) {
      console.log(`+ ${report.source.name} (${rel})${note}: clean`);
      continue;
    }
    const counts = countFindings(report.findings);
    console.log(`x ${report.source.name} (${rel})${note}: ${describeCounts(counts)}`);
    const sorted = [...report.findings].sort(
      (a, b) => SEVERITY_RANK[a.severity] - SEVERITY_RANK[b.severity] || a.nodeId.localeCompare(b.nodeId)
    );
    for (const finding of sorted) printFinding(finding);
  }

  for (const failure of result.failures) {
    const rel = path.relative(process.cwd(), failure.file) || failure.file;
    console.log(`! ${rel}: parse failure — ${failure.error}`);
  }

  const s = result.summary;
  console.log('');
  console.log(
    `Summary: ${s.analyzed} flow(s) analyzed (${s.flows} found) — ` +
      `${s.errors} error(s), ${s.warnings} warning(s), ${s.suggestions} suggestion(s)` +
      (s.parseFailures > 0 ? `, ${s.parseFailures} parse failure(s)` : '')
  );
}

function countFindings(findings: Finding[]): Record<Severity, number> {
  const counts: Record<Severity, number> = { error: 0, warning: 0, suggestion: 0 };
  for (const f of findings) counts[f.severity] += 1;
  return counts;
}

function describeCounts(counts: Record<Severity, number>): string {
  const parts: string[] = [];
  if (counts.error > 0) parts.push(`${counts.error} error(s)`);
  if (counts.warning > 0) parts.push(`${counts.warning} warning(s)`);
  if (counts.suggestion > 0) parts.push(`${counts.suggestion} suggestion(s)`);
  return parts.join(', ') || 'clean';
}

/** Run the doctor and return the process exit code. */
export function flowDoctor(target: string, options: FlowDoctorOptions = {}): number {
  const result = runDoctor(target, { strict: options.strict });

  if (options.json) {
    console.log(
      JSON.stringify(
        {
          target: result.target,
          flows: result.reports.map((r) => ({
            file: r.source.file,
            name: r.source.name,
            format: r.source.format,
            note: r.source.note,
            findings: r.findings,
          })),
          parseFailures: result.failures,
          summary: result.summary,
          exitCode: result.exitCode,
        },
        null,
        2
      )
    );
  } else {
    printHumanReport(result);
  }
  return result.exitCode;
}

/** Print the lowered execution plan; returns the process exit code. */
export function flowExplain(target: string): number {
  const { sources, failures } = discoverFlows(target);
  if (sources.length === 0) {
    for (const failure of failures) {
      console.error(`Error: ${failure.file}: ${failure.error}`);
    }
    if (failures.length === 0) console.error(`No flow definitions found under ${target}.`);
    return 2;
  }
  sources.forEach((source, i) => {
    if (i > 0) console.log('\n' + '─'.repeat(72) + '\n');
    console.log(explainFlow(source));
  });
  for (const failure of failures) {
    console.error(`\nWarning: ${failure.file} could not be parsed: ${failure.error}`);
  }
  return 0;
}
