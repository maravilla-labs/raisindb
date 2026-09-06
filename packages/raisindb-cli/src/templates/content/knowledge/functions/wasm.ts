export function functionsWasmKnowledge(): string {
  return `# WebAssembly Functions (\`language: wasm\`)

A function can be a **WebAssembly component** built locally in Rust, Go or
TypeScript and uploaded as \`main.wasm\`. It runs in wasmtime with the same
\`raisin.*\` API the JavaScript runtime sees.

Choose wasm for a real toolchain (types, packages, a native test runner) or
CPU-bound work. Do NOT choose it to iterate quickly: a wasm function has **no
source on the server** — the artifact is the code, so every change is a local
rebuild plus an upload. Editing in the admin console is JavaScript's advantage.

## .node.yaml

\`\`\`yaml
node_type: raisin:Function
properties:
  name: greet
  title: Greet
  language: wasm
  entry_file: main.wasm            # -> the guest handler named "default"
  execution_mode: both
  enabled: true
  resource_limits:
    timeout_ms: 5000
    max_memory_bytes: 67108864
  network_policy:
    http_enabled: false
\`\`\`

## entry_file is name-routed: one artifact, N functions

The component exports exactly ONE WIT function, \`handler(name, input)\`. The
suffix of \`entry_file\` selects which registered handler answers:

| entry_file | handler | artifact |
|---|---|---|
| \`main.wasm\` | \`default\` | the sibling asset |
| \`main.wasm:shout\` | \`shout\` | the sibling asset |
| \`../greet/main.wasm:shout\` | \`shout\` | **another node's** asset |

So several Function nodes can share one uploaded artifact — which is how a
package of TypeScript functions ships ~12 MB instead of ~200 MB. The resolved
path must stay inside the functions workspace; one that escapes it is refused.
The server never validates the handler NAME: an unknown one comes back as an
error listing what the guest registered.

## Package layout

Guest source lives OUTSIDE \`content/\`. \`raisindb sync\` maps every non-YAML
file under \`content/\` to a node, so a \`Cargo.toml\` there would upload as an
asset.

\`\`\`
package/content/functions/lib/<ns>/<name>/.node.yaml   language: wasm
package/content/functions/lib/<ns>/<name>/main.wasm    built artifact (ships)
package/wasm/<ns>/<name>/raisin.build.yaml             how to build it
package/wasm/<ns>/<name>/{Cargo.toml|go.mod|package.json, src/…}
package/.rapignore                                     wasm/
\`\`\`

\`raisin.build.yaml\`:

\`\`\`yaml
lang: rust                       # rust | go | ts
node_dir: ../../../content/functions/lib/demo/greet
artifact: main.wasm
command: cargo build --release --target wasm32-wasip2   # optional
output: target/wasm32-wasip2/release/greet.wasm         # optional
handlers: [default, shout]                              # informational
\`\`\`

## Commands (offline — no server needed)

\`\`\`bash
raisindb create function greet --lang rust|go|ts [--ns demo]
raisindb create function greet-shout --into greet --handler shout   # share the artifact
raisindb function build [path] [--all] [--watch] [--debug]
raisindb function doctor [path] [--json] [--strict]
raisindb deploy ./package --repo myapp --install
\`\`\`

- \`function build\` runs the toolchain, copies the artifact into the Function
  node, prints size + sha256, and lists every node whose \`entry_file\` resolves
  to that artifact.
- \`function doctor\` checks toolchain versions, artifact size against the
  32 MiB server cap, \`entry_file\` resolution, and that the handler name a node
  asks for is actually registered. Exit codes: 0 clean, 1 problems, 2 nothing
  to look at.
- \`raisindb function run\` and \`function test --server\` do not exist yet. Test
  natively (\`cargo test\`, \`go test ./...\`, \`vitest run\` — every SDK ships a
  mock host), then deploy and invoke the function the normal way.

## Handlers

\`\`\`rust
// Rust: raisin-sdk
#[raisin_sdk::handler]                  // "default"
fn greet(input: Input) -> Result<Output> { … }
#[raisin_sdk::handler(name = "shout")]  // "shout"
fn shout(input: Input) -> Result<Output> { … }
raisin_sdk::export!(greet, shout);
\`\`\`

\`\`\`go
// Go: github.com/maravilla-labs/raisindb/sdks/go/raisin (TinyGo, -target=wasip2)
func init() {
    raisin.HandleDefault(greet)
    raisin.Handle("shout", shout)
}
\`\`\`

\`\`\`js
// TypeScript: @raisindb/function-wasm (jco). An ordinary QuickJS function.
export async function handler(input) { … }   // "default"
export async function shout(input) { … }     // "shout"
\`\`\`

## Sandbox — what is NOT there

- No \`wasi:sockets\`, no \`wasi:http\`, no filesystem (zero preopens), no
  environment. Egress is \`raisin.http.*\` only, gated by \`network_policy\`.
- No timers: in TypeScript \`setTimeout\`/\`setInterval\` do nothing useful, and
  \`fetch\` is absent (use \`raisin.http.fetch\`). \`await\` works.
- \`Resource.resize\` / \`toImage\` / \`getPageCount\` are unavailable — they need
  the server's per-execution temp files. Keep such a function in
  \`language: javascript\`.
- \`resource_limits.max_stack_bytes\` and \`max_instructions\` are ignored for
  wasm; \`timeout_ms\` and \`max_memory_bytes\` are enforced.
- Failures are reported as \`TIMEOUT\`, \`MEMORY_LIMIT\`, \`STACK_OVERFLOW\`,
  \`INVALID_OUTPUT\` (the guest returned non-JSON), or a \`wasm trap: …\`
  runtime error with a guest-only backtrace.

An artifact is validated at upload: a core module, a component that does not
export \`handler\`, or one importing \`wasi:sockets\`/\`wasi:http\` is rejected
with HTTP 400 and the reason.
`;
}
