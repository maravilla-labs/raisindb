/**
 * Package folder context: resolve function references to .node.yaml files.
 *
 * Layouts supported (see src/sync/operations.ts and builtin-packages/):
 *   <pkg>/<workspace>/<path...>/.node.yaml          (sync layout)
 *   <pkg>/content/<workspace>/<path...>/.node.yaml  (builtin package layout)
 */

import fs from 'fs';
import path from 'path';
import yaml from 'yaml';

export interface ResolvedFunction {
  file: string;
  nodeType?: string;
  inputSchema?: { required?: unknown; [key: string]: unknown };
}

export class PackageContext {
  private roots: string[];
  private cache = new Map<string, ResolvedFunction | null>();

  constructor(packageDir: string) {
    this.roots = [packageDir, path.join(packageDir, 'content')].filter(
      (p) => fs.existsSync(p) && fs.statSync(p).isDirectory()
    );
  }

  /**
   * Resolve a reference path (e.g. "/lib/charge-payment") in a workspace
   * (default "functions") to a .node.yaml in the package folder.
   * Returns null when no candidate file exists.
   */
  resolve(refPath: string, workspace = 'functions'): ResolvedFunction | null {
    const key = `${workspace}\0${refPath}`;
    if (this.cache.has(key)) return this.cache.get(key)!;

    const relative = refPath.replace(/^\/+/, '');
    let result: ResolvedFunction | null = null;
    for (const root of this.roots) {
      const candidates = [
        path.join(root, workspace, relative, '.node.yaml'),
        path.join(root, workspace, `${relative}.yaml`),
        path.join(root, workspace, `${relative}.node.yaml`),
      ];
      for (const candidate of candidates) {
        if (!fs.existsSync(candidate)) continue;
        result = { file: candidate };
        try {
          const doc = yaml.parse(fs.readFileSync(candidate, 'utf-8')) as Record<string, unknown> | null;
          if (doc && typeof doc === 'object') {
            result.nodeType = typeof doc.node_type === 'string' ? doc.node_type : undefined;
            const props = (doc.properties ?? {}) as Record<string, unknown>;
            if (props.input_schema && typeof props.input_schema === 'object') {
              result.inputSchema = props.input_schema as ResolvedFunction['inputSchema'];
            }
          }
        } catch {
          // unparseable target: keep file-only result (existence is enough
          // for FUNCTION_NOT_FOUND; shape checks are skipped)
        }
        break;
      }
      if (result) break;
    }
    this.cache.set(key, result);
    return result;
  }

  /** Required argument names from the function's input_schema, if declared. */
  static requiredArguments(fn: ResolvedFunction): string[] {
    const required = fn.inputSchema?.required;
    if (!Array.isArray(required)) return [];
    return required.filter((r): r is string => typeof r === 'string');
  }
}
