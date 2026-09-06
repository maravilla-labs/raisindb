# wasm guest fixtures

Seven tiny WebAssembly **components** the `raisin-functions` wasm runtime tests
run for real. They are checked in as built `.wasm` files under
`crates/raisin-functions/src/runtime/wasm/fixtures/` and `include_bytes!`'d, so
`cargo test -p raisin-functions --lib --features wasm` needs no wasm toolchain.

## Rebuild

```sh
rustup target add wasm32-wasip2
cd fixtures/wasm-guests
cargo build --release --target wasm32-wasip2
./copy-fixtures.sh          # or: make wasm-fixtures  (from the repo root)
```

`wasm32-wasip2` emits a component directly (rustc ≥ 1.82) — no `cargo-component`
and no `wasm-tools component new` step.

**Never build these from a `build.rs`.** `crates/raisin-server/build.rs` documents
why a nested cargo invocation inside a build script deadlocks and corrupts the
outer build; the same applies here. This is a `make` target on purpose.

## Why a separate workspace

The repo root excludes `fixtures/*`. These crates build for `wasm32-wasip2` with
their own profile (`opt-level = "s"`, `lto`, `panic = "abort"`, `strip`); inside
the main workspace they would also inherit its `.cargo/config.toml`
`split-debuginfo = "unpacked"`, which does not exist on wasm32.

## The fixtures

| crate | artifact | what it proves |
|---|---|---|
| `echo` | `echo.wasm` | ONE artifact carries N handlers (`default`, `reverse`); an unknown name is a guest `Err` naming the registered set |
| `call-host` | `call_host.wasm` | the generic gateway: a no-arg call, a call with args, and an unknown method that must be `Err` rather than a trap |
| `log` | `log.wasm` | the `log` import at two levels plus stdout/stderr draining |
| `spin` | `spin.wasm` | epoch interruption (never returns) |
| `alloc` | `alloc.wasm` | the store's `ResourceLimiter` (grows 16 MiB at a time) |
| `wrong-world` | `wrong_world.wasm` | a component exporting `run` instead of `handler` is refused |
| `sockets` | `sockets.wasm` | an unlinked `wasi:sockets` import is refused, by name |

The first five generate their bindings from the ONE source of truth,
`crates/raisin-functions/wit/raisin-function.wit` — referenced by path, never
copied, so they cannot drift from the host.
