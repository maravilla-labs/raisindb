---
title: WebAssembly — limits and sandbox
description: What a wasm function may do, what stops it, how failures are reported, and the [functions.wasm] server settings.
---

# Limits and sandbox

A WebAssembly function runs in a **fresh wasmtime store per execution**, built
from a compiled component and dropped when the call returns. Nothing survives a
call and nothing crosses tenants: the only thing shared is the compiled image,
which is code, not data, and is keyed by a hash the server computes over the
artifact bytes — never the node's `content_hash`, which a tenant can write.

## What the guest can reach

| Capability | Linked? | Notes |
|---|---|---|
| `raisin:function/host` (`call`, `log`, `context`, `abi-version`) | **yes** | the only door to the database |
| `wasi:io`, `wasi:clocks`, `wasi:random` | yes | `Date.now()`, `Instant::now()`, seeded hash maps |
| `wasi:cli` (stdout/stderr, environment, exit) | yes | stdout/stderr are captured into the execution logs; the environment is **empty** |
| `wasi:filesystem` | yes, with **zero preopens** | linked only because wasi-libc and StarlingMonkey call `preopens` at startup; there is nothing to open |
| `wasi:sockets` | **no** | a component importing it is rejected at upload, by name |
| `wasi:http` | **no** | egress is `raisin.http.*`, which the function's `network_policy` gates |

Component Model async (WASI 0.3) is not enabled. Every host import is
synchronous from the guest's point of view; the host awaits on its side.

## Resource limits

From the node's `resource_limits`:

| field | effect |
|---|---|
| `timeout_ms` | epoch interruption cuts off **guest** code; an outer wall clock (timeout + 250 ms) covers a host call that hangs |
| `max_memory_bytes` | a refused `memory.grow`; the guest allocator then aborts, and the server reports `MEMORY_LIMIT` rather than the `unreachable` it saw |
| `max_stack_bytes` | **ignored for wasm** — wasmtime fixes the guest stack when the engine is built; use `[functions.wasm] max_wasm_stack_bytes` |
| `max_instructions` | not enforced (fuel is off) |

Concurrency shares the server-wide function execution permit with the
JavaScript and Starlark runtimes. For wasm that permit bounds **memory**
(N concurrent stores × their memory ceiling), not worker threads: nothing in
the wasm path blocks a runtime thread.

## How failures are reported

| What happened | Result |
|---|---|
| guest returned `Ok(json)` | success |
| guest returned `Ok(` non-JSON `)` | `INVALID_OUTPUT`, with a preview of the payload |
| guest returned `Err(msg)` | a runtime failure carrying `msg` |
| the deadline passed (epoch or wall clock) | `TIMEOUT` |
| memory growth was refused | `MEMORY_LIMIT` |
| guest stack exhausted | `STACK_OVERFLOW` |
| any other trap | a runtime failure, `wasm trap: …`, with a **guest-only** backtrace |
| the artifact will not compile or link | a validation error naming the reason |
| stdout / stderr lines | log entries at info / error |

Host stack frames are never rendered into a tenant-visible message.

## Rejected at upload, not at midnight

A `.wasm` uploaded under a `raisin:Function` node — and a `.wasm` inside an
installed package — is compiled and linked before it is kept. Three distinct
refusals:

- *not a valid WebAssembly component* — a core module, or garbage;
- *an import it needs is not provided by this host* — `wasi:sockets` and
  `wasi:http` are named explicitly;
- *does not export `handler`* — the component was built against a different
  world.

A rejected upload answers HTTP 400 with code `INVALID_WASM_COMPONENT`, and
leaves no blob and no node behind. A server built without the `wasm` feature
accepts and logs instead, so a stock build still installs packages.

The handler **name** is never validated by the host. The guest owns its handler
namespace and answers an unknown name with an error listing what it registered;
a host-side allow-list would make a correct guest unreachable.

## Server settings

```toml
[functions.wasm]
enabled = true
max_artifact_bytes = 33554432      # 32 MiB; checked at load, run-file and upload
compiled_cache_bytes = 268435456   # 256 MiB of compiled code, process-wide LRU by weight
max_wasm_stack_bytes = 1048576     # engine-global guest stack ceiling
epoch_tick_ms = 10                 # timeout resolution
allocation = "on-demand"           # or "pooling"
max_instances = 15                 # pooling sizing only
stdout_capture_bytes = 1048576     # per stream, per execution
```

Every key has the default shown; an absent `[functions]` section means exactly
this. `allocation = "pooling"` reserves address space when the engine is built —
faster instantiation, but a sizing mistake fails at **boot**, so `on-demand` is
the default. `enabled = false` still registers the runtime, so a wasm function
fails naming the setting rather than the language.

The artifact cap has one reader in the server, so the load path, the run-file
path and the upload path cannot disagree about it. `raisindb function doctor`
mirrors the 32 MiB default locally; if your operator lowered it, doctor's
verdict is optimistic and the upload is the authority.
