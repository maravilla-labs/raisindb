/**
 * Flow doctor integration for package validation.
 *
 * Discovers every raisin:Flow content node carrying workflow_data in a
 * package file map and runs the flow doctor checks (src/flow/checks.ts)
 * against it, translating findings into the WASM validator's result shape
 * so they merge into the normal package validation summary.
 *
 * Doctor errors fail validation (package create/deploy abort before the
 * .rap is built); warnings and suggestions are reported as warnings.
 */

import yaml from 'yaml';
import { doctorWorkflowData } from './doctor.js';
import { PackageContext } from './package-context.js';
import type { Finding } from './types.js';
import type {
  PackageValidationResults,
  ValidationError,
  ValidationResult,
} from '../wasm/types.js';

/** Translate a doctor finding into the WASM validator error shape. */
function toValidationError(filePath: string, finding: Finding): ValidationError {
  const where = finding.nodeId ? ` @${finding.nodeId}` : '';
  const fieldPath = finding.field
    ? `workflow_data${where} (${finding.field})`
    : `workflow_data${where}`;
  return {
    file_path: filePath,
    field_path: fieldPath,
    error_code: finding.code,
    message: finding.nodeId
      ? `Flow node "${finding.nodeId}": ${finding.message}`
      : finding.message,
    severity: finding.severity === 'error' ? 'error' : 'warning',
    fix_type: 'manual',
  };
}

/**
 * Run the flow doctor against every raisin:Flow node in the package.
 *
 * @param packageDir absolute package directory (for function-ref resolution)
 * @param files map of package-relative path -> YAML content (same map the
 *              WASM validator receives, see collectPackageFiles)
 * @returns per-file results for files that produced doctor findings
 */
export function validatePackageFlows(
  packageDir: string,
  files: Record<string, string>
): PackageValidationResults {
  const pkg = new PackageContext(packageDir);
  const results: PackageValidationResults = {};

  for (const [relPath, text] of Object.entries(files)) {
    let doc: unknown;
    try {
      doc = yaml.parse(text);
    } catch {
      continue; // YAML syntax errors are already reported by the WASM validator
    }
    if (doc == null || typeof doc !== 'object' || Array.isArray(doc)) continue;
    const node = doc as Record<string, unknown>;
    if (node.node_type !== 'raisin:Flow') continue;

    const props = (node.properties ?? {}) as Record<string, unknown>;
    const workflowData = props.workflow_data ?? props.flow_definition;
    if (workflowData == null) continue; // flow node without a definition: nothing to analyze

    const { findings } = doctorWorkflowData(workflowData, { filePath: relPath, pkg });
    if (findings.length === 0) continue;

    const errors: ValidationError[] = [];
    const warnings: ValidationError[] = [];
    for (const finding of findings) {
      const entry = toValidationError(relPath, finding);
      (entry.severity === 'error' ? errors : warnings).push(entry);
    }

    const result: ValidationResult = {
      success: errors.length === 0,
      file_type: 'content',
      errors,
      warnings,
    };
    results[relPath] = result;
  }

  return results;
}

/**
 * Merge flow doctor results into the schema validator results in place.
 * Findings for files the schema validator already reported on are appended.
 */
export function mergeFlowResults(
  base: PackageValidationResults,
  flowResults: PackageValidationResults
): PackageValidationResults {
  for (const [filePath, flowResult] of Object.entries(flowResults)) {
    const existing = base[filePath];
    if (existing) {
      existing.errors.push(...flowResult.errors);
      existing.warnings.push(...flowResult.warnings);
      existing.success = existing.errors.length === 0;
    } else {
      base[filePath] = flowResult;
    }
  }
  return base;
}
