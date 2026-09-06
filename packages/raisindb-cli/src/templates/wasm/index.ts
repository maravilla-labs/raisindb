/**
 * The wasm-function scaffolds, one per guest language.
 *
 * Each emits a toolchain project under `wasm/<ns>/<name>/` and a Function node
 * under `content/functions/lib/<ns>/<name>/`. The split is what keeps `sync`
 * from uploading `Cargo.toml` as an asset: everything under `content/` that is
 * not YAML becomes a node, so source must live outside it (see
 * `sync/mapping.ts`).
 */

import type { FileEntry } from '../types.js';
import { commonFiles, type WasmFnVars } from './shared.js';
import { goFiles } from './go.js';
import { rustFiles } from './rust.js';
import { assemblyScriptFiles } from './assemblyscript.js';
import { tsFiles } from './ts.js';

export * from './shared.js';

/** Every file a scaffold writes, paths relative to the package root. */
export function wasmFunctionFiles(
  vars: WasmFnVars,
  nodePath: string,
  projectPath: string
): FileEntry[] {
  const lang =
    vars.lang === 'rust'
      ? rustFiles(vars, projectPath)
      : vars.lang === 'go'
        ? goFiles(vars, projectPath)
        : vars.lang === 'assemblyscript'
          ? assemblyScriptFiles(vars, projectPath)
          : tsFiles(vars, projectPath);
  return [...commonFiles(vars, nodePath, projectPath), ...lang];
}
