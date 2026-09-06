# The WebAssembly Function ABI

**Audience:** RaisinDB contributors (Rust, and SDK maintainers). This is the
contract between the server's wasm runtime and a guest component, why it has the
shape it has, and what you must not change without changing everything.

The user-facing guides live in
[`docs/website/docs/functions/`](../website/docs/functions/wasm-functions.md);
this one is about the seam.

---

## 1. The contract

One file: `crates/raisin-functions/wit/raisin-function.wit`. Every SDK carries a
byte-identical **copy** (`sdks/*/wit/`), written by the generator and asserted
by `sdk_wit_copies_match_source` in `cargo test -p raisin-functions --lib` — the
toolchains (jco, TinyGo, cargo-component) all want the WIT inside the project,
and a hand-maintained second copy is exactly the mirrored-path bug class
CLAUDE.md warns about.

```wit
package raisin:function@0.1.0;

interface host {
    enum log-level { debug, info, warn, error }
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

`HOST_ABI_VERSION` is `"0.1.0"` (`runtime/wasm/`), answered by `abi-version()`.
Bump it only when the WIT changes shape. Note that the WIT **package** version
is part of the import specifier a guest writes
(`raisin:function/host@0.1.0`) — the TS SDK fails at Wizer time without it —
so a package-version bump is a breaking change for every built artifact.

## 2. Why one generic gateway

`call(method, args-json)` mirrors `__raisin_call` in QuickJS
(`runtime/quickjs/gateway.rs`) and Starlark string-for-string: look the method
up in the registry by `internal_name`, parse `args` as a positional JSON array,
await the descriptor's invoker, return `InvokeResult::to_json_string()`.

The alternative — a WIT function per host method — would put a typed
signature per host method into the world, make every host addition an ABI break, and give the
three SDKs three places to drift. Instead the *typed* surface is generated per
language from the same registry (`make gen-bindings`), so a new
`ApiMethodDescriptor` reaches Rust, Go and TypeScript in one step and
`cargo test -p raisin-functions --lib` fails if the committed output is stale.

Only a **host bug** may trap. An unknown method, bad arguments, or an API error
are all `Ok(Err(message))` — data the guest can act on.

## 3. Why the export is name-routed

`handler(name, input)`, not `handler(input)`.

WIT exports are static, so a dynamic set of handler names is only expressible as
a routed parameter. That buys two things the alternative cannot:

- **One artifact, N handlers.** Decisive for TypeScript: a StarlingMonkey
  component is 8–15 MB, so one artifact per function makes a twenty-function
  package ship ~200 MB.
- **N Function nodes, one artifact.** `entry_file: ../shared/main.wasm:on-order`
  resolves the asset relative to the function node's own path.

`name` arrives already resolved by `execution::entry_file::resolve_entry_file`:
the suffix of `entry_file`, or `"default"` for a bare `main.wasm`. It is passed
into the guest **verbatim**.

> **Never validate the handler name on the host side.** The guest owns its
> handler namespace and answers an unknown name with an `Err` naming what it
> registered. A host-side allow-list would have to track every SDK's
> registration mechanism, and its first error would be to make a correct guest
> unreachable. The regression guard is
> `runtime::wasm::tests::an_unknown_handler_is_the_guests_error_naming_what_it_registered`.

The asset path *is* validated: `resolve_entry_file` refuses a path that climbs
above the functions workspace rather than clamping it, because a clamped
`../../../etc/passwd` resolves to an ordinary node path and reads whatever lives
there.

## 4. The runtime, in one page

`crates/raisin-functions/src/runtime/wasm/`, behind the `wasm` feature
(default-on in `raisin-server`, off in `raisin-functions` so other crates' tests
do not link Cranelift):

| file | what |
|---|---|
| `config.rs` | `WasmRuntimeConfig`, `configure_wasm_runtime()` — installed once, early in `main.rs` |
| `engine.rs` | the process-wide `Engine`, the epoch ticker thread, the `Linker` with the WASI subset |
| `bindings.rs` | `bindgen!`-generated world, `HostState`, `impl host::Host` |
| `compile.rs` | `compile()` — the ONLY compiler; produces the three distinct rejection messages |
| `cache.rs` | blake3-keyed `moka` cache, single-flight, `validate_component`, the artifact cap |
| `limits.rs` | `FunctionLimiter` — records `denied_memory_growth` and `peak_memory` |
| `errors.rs` | trap → `ExecutionError`, guest-only backtraces |
| `runtime_impl.rs` | `impl FunctionRuntime for WasmRuntime` |

Invariants worth knowing before you touch any of it:

- **The cache key is OUR blake3 of the bytes, never the node's `content_hash`.**
  `raisin:Asset` is `strict: false`, so a tenant can write any `content_hash`;
  keying on it would let tenant A poison tenant B's cached artifact.
- **A `Store` is built fresh per execution and dropped at the end.** That is
  what makes four of the seven QuickJS pool invariants structural rather than
  disciplinary. The compiled component is the only shared thing, and it holds no
  state.
- **Compilation happens inside `spawn_blocking`, under a single-flight lock**
  (`KeyedMutex`). Cranelift on a 10 MB jco component takes seconds; eight
  concurrent cold executions must compile once.
- **`compile()` is the single implementation** behind both the cache miss and
  upload-time `validate_component`, so acceptance at upload and acceptance at
  run time cannot drift.
- **Timeout is epoch interruption plus an outer `tokio::time::timeout`.** Epochs
  only cut off guest code; a host call awaiting I/O needs the wall clock. Fuel
  is off.
- **`wasi:sockets` and `wasi:http` are never linked**, so a component importing
  them fails at `instantiate_pre` with the import named — which is the
  upload-time error message. Egress stays `raisin.http.*`, gated by the
  function's `network_policy`.
- **`wasi:filesystem` IS linked, with zero preopens.** wasi-libc and
  StarlingMonkey call `filesystem/preopens` during startup even when they never
  open anything.

## 5. Adding a host method

Follow [Adding a Native Host Capability](./adding-a-native-host-capability.md).
The wasm-specific step is its last one: **run `make gen-bindings` and commit the
result.** Nothing in `runtime/wasm/` needs to change — the gateway is generic —
but the three SDKs' typed wrappers and `raisin.generated.d.ts` are rendered from
the registry, and the freshness test fails until they are regenerated.

If the method needs something the gateway cannot express (a resource handle, a
streaming reply), that is a **WIT change**: bump `HOST_ABI_VERSION`, update the
world, regenerate every SDK, and accept that already-built artifacts must be
rebuilt.

## 6. Fixtures and tests

`fixtures/wasm-guests/` is its own cargo workspace (the root `Cargo.toml`
excludes `fixtures/*` and `sdks/*`), targeting `wasm32-wasip2`. The built
components are committed under `runtime/wasm/fixtures/` and `include_bytes!`'d,
so the runtime unit tests need no wasm toolchain:

```sh
make wasm-fixtures     # rustup target add + cargo build + copy
```

Never build them from a `build.rs` — see `crates/raisin-server/build.rs` for the
nested-build rules.

```sh
cargo test -p raisin-functions --lib --features wasm
SKIP_ADMIN_BUILD=1 cargo test -p raisin-server --bins config
SKIP_ADMIN_BUILD=1 cargo test -p raisin-server --test all wasm_run_file_test -- --ignored --nocapture
```

`raisin-server` is **bin-only** — it declares one `[[bin]]` and has no
`src/lib.rs` — so the `[functions.wasm]` config tests are selected with
`--bins`, not `--lib`. The `--lib` form fails with `no library targets found in
package raisin-server` before running anything.

Linting the wasm module is `--no-deps`:

```sh
cargo clippy -p raisin-functions --features wasm --no-deps -- -D warnings
```

Without `--no-deps`, cargo runs clippy-driver as the workspace wrapper over
*every* workspace crate in the dependency graph and `-D warnings` denies their
pre-existing lints too, so the command reports failures in crates the change
never touched. The first wall is `raisin-storage` (14 `too_many_arguments` on
unmodified committed code); fixing it only moves the wall, because the
workspace carries roughly 305 such findings — making that gate green is a
`[workspace.lints.clippy]` policy decision, not something a feature branch can
close.

Both spellings have been mis-typed once each, so the gate is also a make
target — run it rather than retyping the four commands:

```sh
make wasm-check    # fmt --all --check, clippy --no-deps, both test selections
```

## See also

- [Adding a Native Host Capability](./adding-a-native-host-capability.md)
- `sdks/README.md` — the generated-file discipline across the three SDKs
- [WebAssembly functions](../website/docs/functions/wasm-functions.md) — the
  user-facing guide
