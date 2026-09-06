/**
 * `raisindb create function <name> --into <project>` — the one-artifact,
 * N-functions path.
 *
 * A wasm artifact is expensive (a TypeScript component is 8-15 MiB; a package
 * of twenty would be 200 MiB) and a component can carry any number of handlers
 * behind its single `handler(name, input)` export. So a second function is
 * usually not a second project: it is a second handler in an existing one, plus
 * a Function node whose `entry_file` points at that project's artifact —
 * `../<other>/main.wasm:<handler>`.
 *
 * This module does the source half: register the handler where the language's
 * SDK expects it, add a test, and record the name in `raisin.build.yaml`.
 */

import fs from 'fs';
import path from 'path';
import { camelIdent, snakeIdent } from '../templates/wasm/shared.js';
import { registeredHandlers } from './handlers.js';
import type { WasmProject } from './types.js';

/** What `addHandler` changed. */
export interface AddHandlerResult {
  /** Absolute paths that were rewritten. */
  changed: string[];
  /** Things the caller should mention but that are not failures. */
  warnings: string[];
}

function append(file: string, text: string, result: AddHandlerResult): void {
  const existing = fs.readFileSync(file, 'utf-8');
  fs.writeFileSync(file, `${existing.replace(/\s*$/, '\n')}\n${text}`);
  result.changed.push(file);
}

/** Rust: a `#[handler(name = …)]` function plus an entry in `export!(…)`. */
function addRust(project: WasmProject, handler: string, result: AddHandlerResult): void {
  const file = path.join(project.dir, 'src', 'lib.rs');
  if (!fs.existsSync(file)) throw new Error(`${file} not found — is this a Rust guest project?`);
  const ident = snakeIdent(handler);
  const source = fs.readFileSync(file, 'utf-8');
  const exportRe = /((?:raisin_sdk\s*::\s*)?export!\s*\()([^)]*)(\))/;
  if (!exportRe.test(source)) {
    throw new Error(`${file}: no \`raisin_sdk::export!(…)\` to add "${handler}" to`);
  }
  const withFn = source.replace(
    exportRe,
    (_m, open: string, list: string, close: string) =>
      `${open}${list.trim() ? `${list.trim().replace(/,\s*$/, '')}, ` : ''}${ident}${close}`
  );
  const fn = `/// Handler \`${handler}\`, selected by \`entry_file: …/main.wasm:${handler}\`.
#[raisin_sdk::handler(name = "${handler}")]
pub fn ${ident}(input: Input) -> Result<Output> {
    raisin_sdk::log::info(format!("${handler}: {}", input.name));
    Ok(Output {
        greeting: format!("Hello, {}!", input.name),
        handler: "${handler}".to_string(),
    })
}
`;
  const marker = withFn.indexOf('raisin_sdk::export!');
  const insertAt = marker >= 0 ? marker : withFn.length;
  fs.writeFileSync(file, `${withFn.slice(0, insertAt)}${fn}\n${withFn.slice(insertAt)}`);
  result.changed.push(file);

  const tests = path.join(project.dir, 'tests', 'handlers.rs');
  if (fs.existsSync(tests)) {
    append(
      tests,
      `#[test]
fn the_${ident}_handler_is_the_same_artifact() {
    let (out, _) = with_mock(MockHost::new(), || {
        raisin_dispatch("${handler}", r#"{"name":"Ada"}"#).expect("runs")
    });
    let out: Output = serde_json::from_str(&out).expect("json");
    assert_eq!(out.handler, "${handler}");
}
`,
      result
    );
  }
}

/** Go: a handler function plus a `raisin.Handle` call in `init()`. */
function addGo(project: WasmProject, handler: string, result: AddHandlerResult): void {
  const file = path.join(project.dir, 'main.go');
  if (!fs.existsSync(file)) throw new Error(`${file} not found — is this a Go guest project?`);
  const ident = camelIdent(handler);
  const source = fs.readFileSync(file, 'utf-8');
  const initRe = /func init\(\)\s*\{/;
  if (!initRe.test(source)) {
    throw new Error(`${file}: no \`func init()\` to register "${handler}" in`);
  }
  const registered = source.replace(
    initRe,
    (m) => `${m}\n\traisin.Handle("${handler}", ${ident})`
  );
  fs.writeFileSync(
    file,
    `${registered.replace(/\s*$/, '\n')}
// ${ident} answers the "${handler}" handler, selected by
// entry_file: …/main.wasm:${handler}.
func ${ident}(raw json.RawMessage) (any, error) {
	var in input
	if err := json.Unmarshal(raw, &in); err != nil {
		return nil, fmt.Errorf("invalid input: %w", err)
	}
	if in.Name == "" {
		return nil, fmt.Errorf("input.name is required")
	}
	raisin.Info("${handler}: %s", in.Name)
	return output{Greeting: fmt.Sprintf("Hello, %s!", in.Name), Handler: "${handler}"}, nil
}
`
  );
  result.changed.push(file);

  const tests = path.join(project.dir, 'main_test.go');
  if (fs.existsSync(tests)) {
    append(
      tests,
      `func Test${ident.charAt(0).toUpperCase()}${ident.slice(1)}IsTheSameArtifact(t *testing.T) {
	defer raisintest.New().Install()()

	out, err := raisintest.Invoke("${handler}", map[string]string{"name": "Ada"})
	if err != nil {
		t.Fatalf("invoke: %v", err)
	}
	if !strings.Contains(string(out), \`"handler":"${handler}"\`) {
		t.Fatalf("unexpected output %s", out)
	}
}
`,
      result
    );
  }
}

/** TypeScript: one more exported function — the export name IS the handler. */
function addTs(project: WasmProject, handler: string, result: AddHandlerResult): void {
  const file = ['src/index.js', 'src/index.mjs', 'src/index.ts']
    .map((p) => path.join(project.dir, p))
    .find((p) => fs.existsSync(p));
  if (!file) throw new Error(`no src/index.js in ${project.dir} — is this a JS guest project?`);
  append(
    file,
    `/** Handler \`${handler}\`, selected by \`entry_file: …/main.wasm:${handler}\`. */
export async function ${camelIdent(handler)}(input) {
  const name = input && input.name;
  if (!name) throw new Error('input.name is required');
  console.log('${handler}', name);
  return { greeting: \`Hello, \${name}!\`, handler: '${handler}' };
}
`,
    result
  );
}

/** Record the new handler in `raisin.build.yaml`'s informational list. */
function recordHandler(project: WasmProject, handler: string, result: AddHandlerResult): void {
  const source = fs.readFileSync(project.buildFile, 'utf-8');
  if (/^handlers:/m.test(source)) {
    const updated = source.replace(
      /^handlers:\n((?:\s*-\s*.*\n)*)/m,
      (_m, list: string) => `handlers:\n${list}  - ${handler}\n`
    );
    fs.writeFileSync(project.buildFile, updated);
  } else {
    fs.writeFileSync(
      project.buildFile,
      `${source.replace(/\s*$/, '\n')}handlers:\n  - ${handler}\n`
    );
  }
  result.changed.push(project.buildFile);
}

/**
 * Register `handler` in an existing project.
 *
 * Refuses a name the project already registers: two handlers under one name
 * means one of them is unreachable, with no error anywhere.
 */
export function addHandler(project: WasmProject, handler: string): AddHandlerResult {
  const result: AddHandlerResult = { changed: [], warnings: [] };
  const scan = registeredHandlers(project);
  if (scan.names.includes(handler)) {
    throw new Error(
      `${path.basename(project.dir)} already registers a handler named "${handler}"`
    );
  }
  if (scan.note) result.warnings.push(`could not read existing handlers: ${scan.note}`);

  if (project.spec.lang === 'rust') addRust(project, handler, result);
  else if (project.spec.lang === 'go') addGo(project, handler, result);
  else addTs(project, handler, result);

  recordHandler(project, handler, result);
  return result;
}
