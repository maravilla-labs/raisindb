# RaisinDB WebAssembly guest SDKs

Guest-side SDKs for writing a `raisin:Function` as a WebAssembly **component**
(Component Model + WIT, hosted by wasmtime on the server).

| directory | package | status |
|---|---|---|
| `rust/raisin-sdk` | `raisin-sdk` (crates.io name TBD) | **built** — `cargo test` green, `examples/wasm-functions` builds on it |
| `go/raisin` | `github.com/maravilla-labs/raisindb/sdks/go/raisin` | skeleton — B3 |
| `ts/function-wasm` | `@raisindb/function-wasm` | **built** — jco / ComponentizeJS, `vitest run` green, `examples/wasm-functions/wasm/demo/greet-ts` builds on it |

These directories are **excluded from the root cargo workspace** (root
`Cargo.toml` `[workspace] exclude`): they build for `wasm32-wasip2` with their
own profiles and must not inherit this workspace's `.cargo/config.toml`
(`split-debuginfo` does not exist on wasm32).

## Generated files — do not edit

Every `raisin.*` host method is declared ONCE in
`crates/raisin-functions/src/runtime/bindings/`. The typed per-language
wrappers, the SDK-local copies of the WIT contract, and the TS SDK's copy of
`api_wrapper.js` are rendered from that registry:

```
make gen-bindings          # regenerate (output is committed)
make gen-bindings-check    # fail if the committed output is stale
```

`cargo test -p raisin-functions --lib` asserts the same thing, so a host method
added without regenerating fails the test suite rather than silently missing
from an SDK.

Generated, committed, machine-owned:

- `rust/raisin-sdk/src/generated.rs`
- `go/raisin/generated.go`
- `ts/function-wasm/src/generated/raisin.d.ts`
- `ts/function-wasm/src/generated/api_wrapper.js`
- `*/wit/raisin-function.wit` (copies of `crates/raisin-functions/wit/raisin-function.wit`)

**Do not run `cargo fmt` inside `sdks/rust/raisin-sdk`.** rustfmt would rewrite
`src/generated.rs`, and the freshness test compares that file byte-for-byte
against what the generator emits — a reformat makes it permanently stale.
Format the hand-written files individually instead:

```sh
cd sdks/rust/raisin-sdk
# --config skip_children=true is LOAD-BEARING: without it rustfmt follows
# `mod generated;` out of lib.rs and reformats the generated file anyway.
rustfmt --edition 2021 --config skip_children=true \
    src/lib.rs src/host.rs src/wire.rs src/error.rs src/handler.rs \
    src/context.rs src/log.rs src/transaction.rs src/testing.rs \
    src/__private.rs src/bindings.rs raisin-sdk-macros/src/lib.rs tests/*.rs
```

## The contract

One generic host gateway, and one name-routed export:

```wit
call: func(method: string, args: string) -> result<string, string>;
export handler: func(name: string, input: string) -> result<string, string>;
```

`name` is the handler selected by the Function node's `entry_file` suffix
(`main.wasm:on-order` -> `"on-order"`; a bare `main.wasm` -> `"default"`), so
ONE artifact can carry many handlers and many Function nodes can share one
artifact. Every SDK registers handlers BY NAME and routes inside that single
export; an unknown name answers `Err` listing what the guest registered. The
host never validates the name against an allow-list — the guest owns its
handler namespace.
