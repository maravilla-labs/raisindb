---
title: WebAssembly functions
description: Write a raisin:Function as a WebAssembly component in Rust, Go or TypeScript — the ABI, the node, and the build/test/deploy loop.
---

# WebAssembly functions

A wasm function is an ordinary [`raisin:Function`](./overview.md) whose code is a
**WebAssembly component** (Component Model + WIT) you build locally and upload.
The server runs it with wasmtime, in a fresh sandbox per execution, with the
same `raisin.*` host API the JavaScript and Starlark runtimes see.

There is no source on the server. The unit you ship is `main.wasm`.

## The contract

One WIT file is the single source of truth —
`crates/raisin-functions/wit/raisin-function.wit`. Each SDK carries a
byte-identical copy, kept honest by a freshness test.

```wit
package raisin:function@0.1.0;

interface host {
    enum log-level { debug, info, warn, error }

    /// One generic gateway. `method` is a registry name ("nodes_get",
    /// "http_request", ...); `args` is a JSON array of positional arguments.
    call: func(method: string, args: string) -> result<string, string>;
    log: func(level: log-level, message: string);
    context: func() -> string;
    abi-version: func() -> string;
}

world function {
    import host;
    export handler: func(name: string, input: string) -> result<string, string>;
}
```

Two things follow from that shape, and they are the whole design:

- **One gateway.** Every `raisin.*` method has exactly one implementation on the
  server. The typed per-language wrappers in the SDKs are *generated* from that
  registry (`make gen-bindings`), so a new host capability reaches Rust, Go and
  TypeScript in one step and cannot drift.
- **One export, name-routed.** The export is fixed; the handler *name* is data.
  A guest registers handlers by name and routes inside that single export, so
  **one artifact carries N handlers** and **N Function nodes can share one
  artifact**. The host never validates the name against a list — the guest owns
  its namespace, and answers an unknown name with an error listing what it
  registered.

## The node

```yaml
node_type: raisin:Function
properties:
  name: greet-rust
  title: Greet (Rust / wasm)
  language: wasm
  entry_file: main.wasm            # -> handler "default"
  execution_mode: both
  enabled: true
  resource_limits:
    timeout_ms: 5000
    max_memory_bytes: 67108864
  network_policy:
    http_enabled: false
  input_schema:
    type: object
    properties: { name: { type: string } }
    required: [name]
```

A second node can reuse the same artifact by pointing at it:

```yaml
  name: greet-rust-shout
  language: wasm
  entry_file: ../greet-rust/main.wasm:shout
```

`entry_file` resolves against the function node's path and must stay inside the
functions workspace; a path that escapes it is refused with a validation error.
A bare `main.wasm` selects the handler named `default`.

## Layout of a package

Guest source must stay **outside** `content/`: `raisindb sync` maps every
non-YAML file under `content/` to a node, so a `Cargo.toml` there would upload
as an asset. `raisindb create function` scaffolds this layout:

```
my-package/
  content/functions/lib/<ns>/<name>/.node.yaml   language: wasm
  content/functions/lib/<ns>/<name>/main.wasm    built artifact — the only thing that ships
  wasm/<ns>/<name>/raisin.build.yaml             how to build it
  wasm/<ns>/<name>/{Cargo.toml|go.mod|package.json, src/…}
  .rapignore                                     wasm/
```

`raisin.build.yaml` is what the CLI reads:

```yaml
lang: rust                         # rust | go | ts
node_dir: ../../../content/functions/lib/demo/greet-rust
artifact: main.wasm                # filename inside node_dir
command: cargo build --release --target wasm32-wasip2   # optional; a default per language
output: target/wasm32-wasip2/release/greet_rust.wasm    # optional; a default per language
handlers: [default, shout]         # informational
```

## The dev loop

```bash
# Run these from the package directory — the one holding manifest.yaml.

# 1. Scaffold — a Function node, a toolchain project, and a test
raisindb create function greet --lang rust --ns demo

# 2. Add a SECOND handler backed by the SAME artifact
raisindb create function greet-shout --into greet --handler shout

# 3. Build: run the toolchain, copy the artifact into the Function node
raisindb function build wasm/demo/greet          # or --all, --watch, --debug

# 4. Check the project against the nodes that point at it
raisindb function doctor                         # --json, --strict

# 5. Ship (from the directory above the package)
raisindb deploy ./package --repo myapp --install
```

`function build` prints the artifact size and sha256 and **lists every Function
node whose `entry_file` resolves to that artifact**, so a build that backs five
handlers says so. `function doctor` checks toolchain versions, that the
artifact exists and is under the server's cap, that every `entry_file` resolves
inside the package, and — by scanning the project's registrations — that the
handler name a node asks for is actually registered. Exit codes follow `flow
doctor`: `0` clean, `1` problems, `2` nothing to look at.

:::note
`raisindb function run` and `raisindb function test --server` are not
implemented yet. Until they are, run natively with your own toolchain
(`cargo test`, `go test ./...`, `vitest run` — every SDK ships a mock host, so
handlers are testable with no server), and against a server with
`raisindb deploy --install` followed by
`POST /api/functions/{repo}/{name}/invoke`.
:::

## Running one against a server

Deployed functions are invoked exactly like JavaScript ones — from a trigger, a
workflow step, an AI tool, `POST /api/functions/{repo}/{name}/invoke`, or the
WebSocket API. Nothing about a caller changes because the callee is wasm.

The admin console shows a wasm function's artifact (size, hash, world) instead
of a code editor, with buttons to replace it and to run it.

There is also a direct artifact run for the editor loop:
`POST /api/files/{repo}/run` with `{ node_id, handler, input, timeout_ms }`,
where `node_id` is the **asset** node (`main.wasm`), not the Function node, and
`handler` is passed to the guest verbatim (empty means `default`). It streams
`started` / `log` / `result` / `done` SSE events. Inline `code` is text-only:
upload the artifact and run it by `node_id`.

## What the server does with the artifact

- **Validated at upload.** A `.wasm` uploaded under a `raisin:Function` node is
  compiled and linked before it is kept. A core module, a component that does
  not export `handler`, or one importing something the host does not provide
  (`wasi:sockets`, `wasi:http`) is rejected with HTTP 400 and the reason, and no
  blob or node is left behind. A `.wasm` in an installed package is validated
  the same way.
- **Compiled once.** Compiled components are cached process-wide, keyed by a
  hash **the server computes over the bytes** — never the node's
  `content_hash`, which a tenant can write. Two tenants running byte-identical
  artifacts share one compiled image and still see their own context and input.
- **Fresh sandbox per call.** A new `Store` per execution, dropped at the end:
  no state survives a call, and none crosses tenants.

See [Limits and sandbox](./wasm-limits.md) for the resource and capability
envelope, and the per-language guides for
[Rust](./wasm-rust.md), [Go](./wasm-go.md) and
[TypeScript](./wasm-typescript.md).

The worked example is `examples/wasm-functions/` in the repository: the same
greeting in three languages, plus a fourth Function node that shares the Rust
artifact through `../greet-rust/main.wasm:shout`.
