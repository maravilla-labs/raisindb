/**
 * Go guest scaffold — TinyGo compiles the component; `go test` runs the same
 * handlers natively against the mock host.
 */

import type { FileEntry } from '../types.js';
import { camelIdent, type WasmFnVars } from './shared.js';

const SDK_MODULE = 'github.com/maravilla-labs/raisindb/sdks/go/raisin';

function goMod(v: WasmFnVars): string {
  const replace =
    v.sdk.kind === 'path'
      ? `
// In-repo scaffold: build against the SDK in this checkout rather than a
// published module. Delete this once the SDK is released.
replace ${SDK_MODULE} => ${v.sdk.value}
`
      : '';
  return `module ${v.name}

go 1.22

require ${SDK_MODULE} v0.0.0
${replace}`;
}

function mainGo(v: WasmFnVars): string {
  const ident = camelIdent(v.handler);
  const register =
    v.handler === 'default'
      ? `raisin.HandleDefault(${ident})`
      : `raisin.Handle("${v.handler}", ${ident})`;
  return `// Command ${v.name} is a RaisinDB function compiled to a WebAssembly component.
//
// Handlers register BY NAME into the one exported handler(name, input), so one
// main.wasm can back several raisin:Function nodes:
//
//	entry_file: main.wasm              -> "default"
//	entry_file: main.wasm:shout        -> "shout"
//	entry_file: ../${v.name}/main.wasm:shout   (from a sibling node directory)
package main

import (
	"encoding/json"
	"fmt"

	"${SDK_MODULE}"
)

// input is what the caller sends.
type input struct {
	Name string \`json:"name"\`
}

// output is what the handler returns.
type output struct {
	Greeting string \`json:"greeting"\`
	Handler  string \`json:"handler"\`
}

func init() {
	${register}
}

// main is required by Go but never runs: the host calls the component export,
// not a program entry point.
func main() {}

// ${ident} ${v.description}
func ${ident}(raw json.RawMessage) (any, error) {
	var in input
	if err := json.Unmarshal(raw, &in); err != nil {
		return nil, fmt.Errorf("invalid input: %w", err)
	}
	if in.Name == "" {
		return nil, fmt.Errorf("input.name is required")
	}
	raisin.Info("greeting %s", in.Name)
	return output{Greeting: fmt.Sprintf("Hello, %s!", in.Name), Handler: "${v.handler}"}, nil
}
`;
}

function mainTestGo(v: WasmFnVars): string {
  return `package main

import (
	"strings"
	"testing"

	"${SDK_MODULE}/raisintest"
)

func TestHandler(t *testing.T) {
	defer raisintest.New().Install()()

	out, err := raisintest.Invoke("${v.handler}", map[string]string{"name": "Ada"})
	if err != nil {
		t.Fatalf("invoke: %v", err)
	}
	if !strings.Contains(string(out), \`"greeting":"Hello, Ada!"\`) {
		t.Fatalf("unexpected output %s", out)
	}
}

func TestMissingNameIsAnError(t *testing.T) {
	defer raisintest.New().Install()()

	if _, err := raisintest.Invoke("${v.handler}", map[string]string{}); err == nil {
		t.Fatal("expected input.name to be required")
	}
}
`;
}

function readme(v: WasmFnVars): string {
  return `# ${v.title}

Go (TinyGo) guest for the \`${v.name}\` RaisinDB function.

    raisindb function doctor .      # toolchain, entry_file, handler names
    go test ./...                   # native tests, no server
    raisindb function build .       # TinyGo build + copy to ${v.nodeDirRel}

TinyGo ≥ 0.34 is required for \`-target=wasip2\`. \`go test\` uses the native
build tag and the mock host, so the ordinary Go toolchain is enough for tests.

## One artifact, many functions

    raisindb create function ${v.name}-shout --lang go --into ${v.name} --handler shout

writes a second Function node with \`entry_file: ../${v.name}/main.wasm:shout\`
and registers \`shout\` here — one component, two functions.
`;
}

/** Every file the Go scaffold writes, under `projectPath`. */
export function goFiles(v: WasmFnVars, projectPath: string): FileEntry[] {
  return [
    { path: `${projectPath}/go.mod`, content: goMod(v) },
    { path: `${projectPath}/main.go`, content: mainGo(v) },
    { path: `${projectPath}/main_test.go`, content: mainTestGo(v) },
    {
      path: `${projectPath}/tests/server.json`,
      content: `${JSON.stringify(
        [{ handler: v.handler, input: { name: 'Ada' }, expect: { greeting: 'Hello, Ada!' } }],
        null,
        2
      )}\n`,
    },
    { path: `${projectPath}/README.md`, content: readme(v) },
  ];
}
