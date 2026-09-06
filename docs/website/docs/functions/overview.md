---
title: Server functions
description: What a raisin:Function is, the four languages it can be written in, and how to choose between them.
---

# Server functions

A **server function** is a node — `node_type: raisin:Function` — living in the
`functions` workspace. It has code, a JSON input and output schema, resource
limits and a network policy, and it runs inside the server: from a trigger, from
a workflow step, as an AI tool, or because someone invoked it over HTTP or
WebSocket.

```yaml
node_type: raisin:Function
properties:
  name: greet
  title: Greet
  language: javascript        # javascript | starlark | sql | wasm
  entry_file: index.js:handler
  execution_mode: both        # async | sync | both
  enabled: true
  resource_limits: { timeout_ms: 5000, max_memory_bytes: 67108864 }
  network_policy: { http_enabled: false }
```

## Four languages, one host API

Whatever the language, a function reaches the database through the same host
surface. Every `raisin.*` method is declared **once**, in the server's binding
registry, and each runtime is a different door onto that one implementation.

| `language` | Runtime | Code lives | Best at |
|---|---|---|---|
| `javascript` | QuickJS | `index.js` beside the node | the default: quick to edit, editable in the admin console |
| `starlark` (alias `python`) | Starlark | `index.py` beside the node | deterministic, Python-shaped configuration logic |
| `sql` | the SQL engine | inline | a query with no procedural logic around it |
| `wasm` | wasmtime (Component Model) | an uploaded `main.wasm` artifact | CPU-bound work, existing Rust/Go libraries, strong typing |

## `entry_file` selects the code *and* the entry point

The grammar is `<file>[:<name>]`, resolved against the function node's own path:

| `entry_file` | file | entry point |
|---|---|---|
| `index.js:handler` | sibling `index.js` | the exported `handler` |
| `index.js` | sibling `index.js` | `handler` (the default for text languages) |
| `main.wasm` | sibling `main.wasm` | the guest handler named `default` |
| `main.wasm:on-order` | sibling `main.wasm` | the guest handler named `on-order` |
| `../shared/main.wasm:on-order` | a **sibling node's** artifact | `on-order` |

The path may climb with `..` as long as it stays inside the functions
workspace; one that would escape it is refused with a validation error rather
than silently clamped.

## When to reach for WebAssembly

Pick `wasm` when you want a real toolchain — a type system, a package
ecosystem, a native test runner — or when the work is CPU-bound enough that a
JIT-less JavaScript interpreter shows. Pick JavaScript when you want to open
the admin console and edit the code in place: a wasm function has **no source
on the server**, only the artifact you built and uploaded.

- [WebAssembly functions](./wasm-functions.md) — the ABI, the node, the dev loop
- [Rust](./wasm-rust.md) · [Go](./wasm-go.md) · [TypeScript](./wasm-typescript.md)
- [Limits and sandbox](./wasm-limits.md) — what a component may and may not do
