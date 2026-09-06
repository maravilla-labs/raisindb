/**
 * The wasm half of package validation (`raisindb package validate|create`,
 * and therefore `deploy` too).
 *
 * The schema validator sees YAML only, so a `language: wasm` node whose
 * `entry_file` points at an artifact nobody built passes it and ships a package
 * that installs a Function with no code. Three things are cheap to check here
 * and expensive to discover on a server: the artifact exists, it stays inside
 * the functions workspace, and it is within the upload cap.
 *
 * Same shape and seam as `wasm/bundled-resource-validator.ts` — it reads the
 * filesystem (artifacts are not in the YAML-only file map) and its results are
 * merged into the normal validation summary. Toolchains are deliberately NOT
 * probed: packing must work on a machine with no cargo, TinyGo or Node build
 * step, because the artifact is already built by then.
 */

import fs from 'fs';
import path from 'path';
import type { PackageValidationResults, ValidationError } from '../wasm/types.js';
import { discoverFunctionNodes, functionsRoot } from './discover.js';
import { formatBytes } from './build.js';
import { MAX_ARTIFACT_BYTES } from './types.js';

function error(file: string, code: string, message: string): ValidationError {
  return {
    file_path: file,
    field_path: 'properties.entry_file',
    error_code: code,
    message,
    severity: 'error',
    fix_type: 'manual',
  };
}

/**
 * Check every `language: wasm` Function node under a package directory.
 *
 * `packageDir` is the package root (the directory holding `manifest.yaml`).
 */
export function validateWasmFunctions(packageDir: string): PackageValidationResults {
  const root = functionsRoot(packageDir);
  const results: PackageValidationResults = {};

  for (const node of discoverFunctionNodes(packageDir)) {
    if (node.language !== 'wasm') continue;
    const rel = path.relative(packageDir, node.file) || node.file;
    const errors: ValidationError[] = [];

    if (!node.entryFile) {
      errors.push(
        error(
          rel,
          'WASM_ENTRY_FILE_MISSING',
          "language is 'wasm' but entry_file is not set. Point it at the artifact " +
            "beside this node, e.g. `entry_file: main.wasm` (or `main.wasm:on-order` " +
            'to select a named handler).'
        )
      );
    } else if (node.escapes) {
      errors.push(
        error(
          rel,
          'WASM_ENTRY_FILE_ESCAPES',
          `entry_file '${node.entryFile}' resolves outside the functions workspace ` +
            `(${path.relative(packageDir, root)}). The server refuses such a path at load; ` +
            'keep the artifact under content/functions/.'
        )
      );
    } else {
      const artifact = node.artifactPath as string;
      const relArtifact = path.relative(packageDir, artifact);
      if (!fs.existsSync(artifact)) {
        errors.push(
          error(
            rel,
            'WASM_ARTIFACT_MISSING',
            `entry_file '${node.entryFile}' points at '${relArtifact}', which is not in ` +
              'the package. Build it first: `raisindb function build`.'
          )
        );
      } else {
        const size = fs.statSync(artifact).size;
        if (size > MAX_ARTIFACT_BYTES) {
          errors.push(
            error(
              rel,
              'WASM_ARTIFACT_TOO_LARGE',
              `'${relArtifact}' is ${formatBytes(size)}, over the ` +
                `${formatBytes(MAX_ARTIFACT_BYTES)} server artifact cap — the upload will be refused.`
            )
          );
        }
      }
    }

    if (errors.length > 0) {
      results[rel] = { success: false, file_type: 'content', errors, warnings: [] };
    }
  }
  return results;
}
