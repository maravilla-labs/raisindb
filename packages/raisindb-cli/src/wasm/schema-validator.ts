/**
 * WASM-based schema validator for RaisinDB packages
 */

import * as fs from 'fs';
import * as path from 'path';
import type {
  PackageValidationResults,
  ValidationError,
  ValidationResult,
} from './types.js';
import {
  partitionTranslationFiles,
  validateTranslationFiles,
} from './translation-validator.js';
import { validateBundledResources } from './bundled-resource-validator.js';
import { validateWasmFunctions } from '../wasm-fn/package-check.js';
import { mergeFlowResults, validatePackageFlows } from '../flow/package-doctor.js';
import {
  EnvContext,
  UnresolvedToken,
  emptyEnvContext,
  substituteEnvTokens,
} from '../env/substitute.js';

// Dynamic import for WASM module (nodejs target)
let wasmModule: typeof import('@raisindb/schema-wasm') | null = null;
let wasmReady = false;

/**
 * Initialize the WASM module
 */
export async function initSchemaValidator(): Promise<void> {
  if (wasmReady) return;

  try {
    // Dynamic import of the WASM module (nodejs target - no async init needed)
    wasmModule = await import('@raisindb/schema-wasm');

    // Initialize panic hooks
    wasmModule.init();

    wasmReady = true;
  } catch (error) {
    throw new Error(`Failed to initialize schema validator: ${error}`);
  }
}

/**
 * Get list of built-in node type names
 */
export async function getBuiltinNodeTypes(): Promise<string[]> {
  await initSchemaValidator();
  return wasmModule!.get_builtin_node_types() as string[];
}

/**
 * Get list of built-in workspace names
 */
export async function getBuiltinWorkspaces(): Promise<string[]> {
  await initSchemaValidator();
  return wasmModule!.get_builtin_workspaces() as string[];
}

/**
 * Validate a manifest.yaml file
 */
export async function validateManifest(
  yaml: string,
  filePath: string
): Promise<ValidationResult> {
  await initSchemaValidator();
  return wasmModule!.validate_manifest(yaml, filePath) as ValidationResult;
}

/**
 * Validate a node type YAML file
 */
export async function validateNodeType(
  yaml: string,
  filePath: string,
  packageNodeTypes: string[] = []
): Promise<ValidationResult> {
  await initSchemaValidator();
  return wasmModule!.validate_nodetype(yaml, filePath, packageNodeTypes) as ValidationResult;
}

/**
 * Validate a workspace YAML file
 */
export async function validateWorkspace(
  yaml: string,
  filePath: string,
  packageNodeTypes: string[] = [],
  packageWorkspaces: string[] = []
): Promise<ValidationResult> {
  await initSchemaValidator();
  return wasmModule!.validate_workspace(
    yaml,
    filePath,
    packageNodeTypes,
    packageWorkspaces
  ) as ValidationResult;
}

/**
 * Validate a content YAML file
 */
export async function validateContent(
  yaml: string,
  filePath: string,
  packageNodeTypes: string[] = [],
  packageWorkspaces: string[] = []
): Promise<ValidationResult> {
  await initSchemaValidator();
  return wasmModule!.validate_content(
    yaml,
    filePath,
    packageNodeTypes,
    packageWorkspaces
  ) as ValidationResult;
}

/**
 * Validate an archetype YAML file
 *
 * Uses serde deserialization as the single source of truth - if it can
 * be parsed into the Archetype struct from raisin-models, it's valid.
 */
export async function validateArchetype(
  yaml: string,
  filePath: string
): Promise<ValidationResult> {
  await initSchemaValidator();
  return wasmModule!.validate_archetype(yaml, filePath) as ValidationResult;
}

/**
 * Validate an element type YAML file
 *
 * Uses serde deserialization as the single source of truth - if it can
 * be parsed into the ElementType struct from raisin-models, it's valid.
 */
export async function validateElementType(
  yaml: string,
  filePath: string
): Promise<ValidationResult> {
  await initSchemaValidator();
  return wasmModule!.validate_elementtype(yaml, filePath) as ValidationResult;
}

/**
 * Validate an entire package directory
 */
export async function validatePackage(
  files: Record<string, string>
): Promise<PackageValidationResults> {
  await initSchemaValidator();
  return wasmModule!.validate_package(files) as PackageValidationResults;
}

/**
 * Apply a fix to YAML content
 */
export async function applyFix(
  yaml: string,
  error: ValidationError,
  newValue?: string
): Promise<string> {
  await initSchemaValidator();
  const result = wasmModule!.apply_fix(yaml, error, newValue);

  if (typeof result === 'object' && 'Err' in result) {
    throw new Error(result.Err as string);
  }

  if (typeof result === 'object' && 'Ok' in result) {
    return result.Ok as string;
  }

  return result as string;
}

/**
 * Read all YAML files from a package directory and return as a map.
 *
 * `{env:NAME}` tokens are resolved as the files are read, so the validator sees
 * exactly the bytes that `package create` will ship. Tokens that resolve to
 * nothing are collected in `unresolved` (keyed by relative path) and reported as
 * validation errors by the caller — they must never be validated as literals and
 * then silently packed.
 */
export function collectPackageFiles(
  packageDir: string,
  env: EnvContext = emptyEnvContext(),
  unresolved?: Record<string, UnresolvedToken[]>
): Record<string, string> {
  const files: Record<string, string> = {};

  function walkDir(dir: string, prefix: string = '') {
    const entries = fs.readdirSync(dir, { withFileTypes: true });

    for (const entry of entries) {
      const fullPath = path.join(dir, entry.name);
      const relativePath = prefix ? `${prefix}/${entry.name}` : entry.name;

      if (entry.isDirectory()) {
        walkDir(fullPath, relativePath);
      } else if (entry.isFile() && /\.ya?ml$/i.test(entry.name)) {
        try {
          const raw = fs.readFileSync(fullPath, 'utf-8');
          const { text, unresolved: missing } = substituteEnvTokens(raw, env);
          files[relativePath] = text;
          if (unresolved && missing.length > 0) {
            unresolved[relativePath] = missing;
          }
        } catch (error) {
          // Skip files that can't be read
        }
      }
    }
  }

  walkDir(packageDir);
  return files;
}

/**
 * Turn unresolved `{env:...}` tokens into validation errors, so a missing
 * variable is discovered by `package validate` with a file and line rather than
 * as a mysterious YAML value later.
 */
function envTokenResults(
  unresolved: Record<string, UnresolvedToken[]>
): PackageValidationResults {
  const results: PackageValidationResults = {};

  for (const [filePath, tokens] of Object.entries(unresolved)) {
    results[filePath] = {
      success: false,
      file_type: 'content',
      errors: tokens.map((token) => ({
        file_path: filePath,
        line: token.line,
        column: token.column,
        field_path: '',
        error_code: 'UNRESOLVED_ENV_TOKEN',
        message:
          `${token.raw} could not be resolved. Set ${token.name} in the environment ` +
          `or a .env file, or give it an inline default: {env:${token.name}:-fallback}`,
        severity: 'error' as const,
        fix_type: 'manual' as const,
      })),
      warnings: [],
    };
  }

  return results;
}

/**
 * Validate a package directory.
 *
 * Runs the WASM schema validator, the translation validator, and the flow
 * doctor (static analysis of every raisin:Flow node's workflow_data) in a
 * single pass; flow doctor errors fail validation like schema errors.
 */
export async function validatePackageDirectory(
  packageDir: string,
  env: EnvContext = emptyEnvContext()
): Promise<PackageValidationResults> {
  const unresolved: Record<string, UnresolvedToken[]> = {};
  const files = collectPackageFiles(packageDir, env, unresolved);
  const { translationFiles, nonTranslationFiles } = partitionTranslationFiles(files);
  const wasmResults = await validatePackage(nonTranslationFiles);
  const translationResults = validateTranslationFiles(translationFiles, files);
  const flowResults = validatePackageFlows(packageDir, files);
  // Bundled-binary presence check reads the filesystem directly (binaries
  // aren't in the YAML-only `files` map), so it takes packageDir.
  const bundledResults = validateBundledResources(packageDir);
  const merged = mergeFlowResults(
    { ...wasmResults, ...translationResults },
    flowResults
  );
  const withBundled = mergeFlowResults(merged, bundledResults);
  // `language: wasm` nodes: the artifact their entry_file names must exist, sit
  // inside the functions workspace, and fit the server's upload cap. Like the
  // bundled-binary check this reads the filesystem, not the YAML file map.
  const withWasm = mergeFlowResults(withBundled, validateWasmFunctions(packageDir));
  return mergeFlowResults(withWasm, envTokenResults(unresolved));
}

/**
 * Get summary statistics from validation results
 */
export function getValidationSummary(results: PackageValidationResults): {
  totalFiles: number;
  errorCount: number;
  warningCount: number;
  hasErrors: boolean;
  filesWithErrors: string[];
  filesWithWarnings: string[];
} {
  const filesWithErrors: string[] = [];
  const filesWithWarnings: string[] = [];
  let errorCount = 0;
  let warningCount = 0;

  for (const [filePath, result] of Object.entries(results)) {
    errorCount += result.errors.length;
    warningCount += result.warnings.length;

    if (result.errors.length > 0) {
      filesWithErrors.push(filePath);
    }
    if (result.warnings.length > 0) {
      filesWithWarnings.push(filePath);
    }
  }

  return {
    totalFiles: Object.keys(results).length,
    errorCount,
    warningCount,
    hasErrors: errorCount > 0,
    filesWithErrors,
    filesWithWarnings,
  };
}

/**
 * Get all fixable errors from validation results
 */
export function getFixableErrors(
  results: PackageValidationResults
): ValidationError[] {
  const fixable: ValidationError[] = [];

  for (const result of Object.values(results)) {
    for (const error of result.errors) {
      if (error.fix_type === 'auto_fixable' || error.fix_type === 'needs_input') {
        fixable.push(error);
      }
    }
  }

  return fixable;
}
