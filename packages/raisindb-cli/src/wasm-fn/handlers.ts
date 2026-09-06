/**
 * Reading the handler names a project registers, straight out of its source.
 *
 * A wasm artifact carries N handlers behind one WIT export, and the Function
 * node's `entry_file` suffix picks one BY NAME. Nothing checks that name until
 * the function is invoked — the host must never keep an allow-list, because the
 * guest owns its handler namespace — so a typo in `entry_file` is otherwise a
 * runtime-only failure on a deployed server. `doctor` closes that gap locally
 * by reading the registrations the SDKs make explicit:
 *
 * | language | registration                                   |
 * |----------|------------------------------------------------|
 * | rust     | `#[raisin_sdk::handler(name = "…")]` + `export!(…)` |
 * | go       | `raisin.Handle("…", fn)` / `raisin.HandleDefault(fn)` |
 * | ts       | the module's exported function names (`handler` = default) |
 *
 * The parse is deliberately syntactic and forgiving: an empty result means
 * "could not tell", never "registers nothing", and `doctor` reports it as a
 * hint rather than an error.
 */

import fs from 'fs';
import path from 'path';
import { DEFAULT_HANDLER, type WasmLang, type WasmProject } from './types.js';

/** What a source scan could work out. */
export interface HandlerScan {
  /** Handler names in `entry_file` spelling, sorted, deduplicated. */
  names: string[];
  /** Files that were read. */
  files: string[];
  /** Set when the scan found no registration mechanism it understands. */
  note?: string;
}

const SKIP_DIRS = new Set(['node_modules', 'target', '.git', 'dist', 'build', 'test', 'tests']);

/** Collect source files under `dir` whose extension is in `exts`. */
function sources(dir: string, exts: string[], out: string[] = []): string[] {
  let entries: fs.Dirent[];
  try {
    entries = fs.readdirSync(dir, { withFileTypes: true });
  } catch {
    return out;
  }
  for (const entry of entries) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      if (SKIP_DIRS.has(entry.name)) continue;
      sources(full, exts, out);
    } else if (exts.includes(path.extname(entry.name))) {
      out.push(full);
    }
  }
  return out.sort();
}

function read(files: string[]): string {
  return files.map((f) => fs.readFileSync(f, 'utf-8')).join('\n');
}

/**
 * Rust: the exported set is `export!(a, b)`, and each identifier's name is its
 * `#[handler]` attribute (`name = "…"`, else `default`).
 *
 * Both halves are needed. The attribute alone would count a handler the crate
 * never exports; the `export!` list alone gives identifiers, not names.
 */
function scanRust(files: string[]): HandlerScan {
  const text = read(files);
  const exportMatch = text.match(/(?:raisin_sdk\s*::\s*)?export!\s*\(([^)]*)\)/);
  if (!exportMatch) {
    return { names: [], files, note: 'no `raisin_sdk::export!(…)` found' };
  }
  const idents = exportMatch[1]
    .split(',')
    .map((s) => s.trim())
    .filter(Boolean);
  const names: string[] = [];
  for (const ident of idents) {
    const attr = new RegExp(
      `#\\[(?:raisin_sdk\\s*::\\s*)?handler(\\s*\\([^)]*\\))?\\]\\s*(?:pub\\s+)?(?:async\\s+)?fn\\s+${ident}\\b`
    ).exec(text);
    const args = attr && attr[1] ? attr[1] : '';
    const named = args.match(/name\s*=\s*"([^"]+)"/);
    names.push(named ? named[1] : DEFAULT_HANDLER);
  }
  return { names, files };
}

/** Go: `raisin.Handle("name", …)` plus `raisin.HandleDefault(…)`. */
function scanGo(files: string[]): HandlerScan {
  const text = read(files);
  const names: string[] = [];
  for (const m of text.matchAll(/raisin\.Handle\s*\(\s*"([^"]+)"/g)) names.push(m[1]);
  if (/raisin\.HandleDefault\s*\(/.test(text)) names.push(DEFAULT_HANDLER);
  if (names.length === 0) {
    return { names, files, note: 'no `raisin.Handle` / `raisin.HandleDefault` call found' };
  }
  return { names, files };
}

/**
 * TypeScript/JavaScript: the module's exported functions.
 *
 * `handler` is the default export name — the same `index.js:handlerName`
 * grammar QuickJS functions already use — so it is reported as `default`.
 */
function scanTs(files: string[]): HandlerScan {
  const text = read(files);
  const names = new Set<string>();
  for (const m of text.matchAll(/export\s+(?:async\s+)?function\s+([A-Za-z_$][\w$]*)/g)) {
    names.add(m[1] === 'handler' ? DEFAULT_HANDLER : m[1]);
  }
  for (const m of text.matchAll(/export\s+(?:const|let|var)\s+([A-Za-z_$][\w$]*)\s*=/g)) {
    names.add(m[1] === 'handler' ? DEFAULT_HANDLER : m[1]);
  }
  if (names.size === 0) return { names: [], files, note: 'no exported functions found' };
  return { names: [...names], files };
}

/** Source files a language's scan reads, relative to the project directory. */
function sourceFiles(lang: WasmLang, dir: string): string[] {
  switch (lang) {
    case 'rust':
      return sources(path.join(dir, 'src'), ['.rs']);
    case 'go':
      return sources(dir, ['.go']).filter((f) => !f.endsWith('_test.go'));
    case 'ts':
      return sources(path.join(dir, 'src'), ['.js', '.mjs', '.ts']);
  }
}

/** Scan a project for the handler names it registers. */
export function registeredHandlers(project: WasmProject): HandlerScan {
  const files = sourceFiles(project.spec.lang, project.dir);
  if (files.length === 0) {
    return { names: [], files, note: 'no source files found to scan' };
  }
  const scan =
    project.spec.lang === 'rust'
      ? scanRust(files)
      : project.spec.lang === 'go'
        ? scanGo(files)
        : scanTs(files);
  return { ...scan, names: [...new Set(scan.names)].sort() };
}
