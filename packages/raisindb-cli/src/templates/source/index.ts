/**
 * Scaffolds for the SOURCE-shipping runtimes: QuickJS (`js`) and Starlark.
 *
 * Deliberately much smaller than the wasm scaffolds, because there is much
 * less to set up: no toolchain, no `raisin.build.yaml`, no separate project
 * tree. The source IS the deliverable, so it lives under `content/` next to
 * its `.node.yaml` and `raisindb sync` pushes it as the function's code asset.
 *
 * They exist so `raisindb create function` is how you start ANY function.
 * Before this, a WebAssembly function had a scaffold and a JavaScript one had
 * a docs page telling you which two files to write by hand.
 */

import type { FileEntry } from '../types.js';
import { SOURCE_ENTRY_FILE, SOURCE_NODE_LANGUAGE, type SourceLang } from '../../wasm-fn/types.js';

export interface SourceFnVars {
  /** Function slug — the node name and directory name. */
  name: string;
  /** Namespace segment under `content/functions/lib/`. */
  ns: string;
  /** `js` or `starlark`. */
  lang: SourceLang;
  /** Exported handler name; `handler` unless the author chose otherwise. */
  handler: string;
  /** One-line description for the Function node. */
  description: string;
}

function nodeYaml(v: SourceFnVars): string {
  const entry = `${SOURCE_ENTRY_FILE[v.lang]}:${v.handler}`;
  return `# ${v.name} — a ${SOURCE_NODE_LANGUAGE[v.lang]} function.
#
# \`entry_file\` is \`<file>:<handler>\`: the code asset beside this node, and
# the function within it to call. Several nodes may point at different
# handlers of the same file.
node_type: raisin:Function
properties:
  title: ${v.name}
  name: ${v.name}
  description: ${JSON.stringify(v.description)}
  language: ${SOURCE_NODE_LANGUAGE[v.lang]}
  entry_file: ${entry}
  execution_mode: both
  enabled: true
  resource_limits:
    timeout_ms: 30000
    # Bytes, not megabytes — an unknown key is silently ignored.
    max_memory_bytes: 134217728
  network_policy:
    http_enabled: false
`;
}

function jsSource(v: SourceFnVars): string {
  return `/**
 * ${v.name} — ${v.description}
 *
 * Runs on the QuickJS runtime. The \`raisin.*\` API is identical to the one a
 * WebAssembly function sees; see https://raisindb.dev/docs/reference/function-api.
 */
export function ${v.handler}(input) {
  console.log(\`greeting \${input.name}\`);

  // Every raisin.* call is synchronous here.
  const children = raisin.nodes.getChildren('content', '/pages', 50);

  return {
    greeting: \`Hello, \${input.name}\`,
    pages: children.length,
  };
}
`;
}

function starlarkSource(v: SourceFnVars): string {
  return `# ${v.name} — ${v.description}
#
# Runs on the Starlark runtime. Method names are snake_case here and camelCase
# in JavaScript; they are the same underlying API.
#
# Note: Starlark enforces no CPU or memory limit of its own, so keep handlers
# short. For anything compute-heavy prefer WebAssembly.

def ${v.handler}(input):
    print("greeting " + input["name"])

    children = raisin.nodes.get_children("content", "/pages", 50)

    return {
        "greeting": "Hello, " + input["name"],
        "pages": len(children),
    }
`;
}

/** The files a source-language function scaffolds, relative to the package root. */
export function sourceFunctionFiles(v: SourceFnVars, nodePath: string): FileEntry[] {
  const entry = SOURCE_ENTRY_FILE[v.lang];
  return [
    { path: `${nodePath}/.node.yaml`, content: nodeYaml(v) },
    {
      path: `${nodePath}/${entry}`,
      content: v.lang === 'js' ? jsSource(v) : starlarkSource(v),
    },
  ];
}
