# raisin-sdk — Rust guest SDK

Write a RaisinDB server function as a WebAssembly **component** in Rust.

```rust
use raisin_sdk::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)] struct Input { name: String }
#[derive(Serialize)]   struct Output { greeting: String }

#[raisin_sdk::handler]                   // handler name "default"
fn greet(input: Input) -> Result<Output> {
    raisin_sdk::log::info(format!("greeting {}", input.name));
    let people = raisin_sdk::nodes::get_children("content", "/people", None)?;
    Ok(Output { greeting: format!("Hello, {} ({} people)", input.name, people.len()) })
}

#[raisin_sdk::handler(name = "shout")]   // handler name "shout"
fn shout(input: Input) -> Result<Output> {
    Ok(Output { greeting: format!("HELLO, {}!", input.name.to_uppercase()) })
}

raisin_sdk::export!(greet, shout);
```

## One artifact, N handlers

The WIT export is fixed — `handler(name: string, input: string)` — and the
handler *name* is data. A Function node's `entry_file` picks it:

| `entry_file` | handler |
|---|---|
| `main.wasm` | `default` |
| `main.wasm:shout` | `shout` |
| `../greet-rust/main.wasm:shout` | `shout`, from a **sibling node's** artifact |

So one uploaded `main.wasm` can back many Function nodes. The host never
validates the name against a list; `export!`'s dispatch table answers an unknown
one with `Err("unknown handler 'x'; registered: default, shout")`.

## Registration: an explicit list, not a link-time inventory

`#[handler]` names a handler and wraps it in its JSON envelope. `export!` lists
the handlers, builds the dispatch table, and — on wasm targets only — emits the
component's single export.

The list is explicit on purpose. `inventory` and `linkme` collect registrations
through constructors or linker sections; on `wasm32-wasip2` a slice that came
back empty would be indistinguishable from a component that registered nothing,
and the failure would only show up at run time on a server. A handler missing
from `export!` is a compile error instead. It also means `export!` is the one
place to read to know what an artifact answers to — which is what
`raisindb function doctor` checks an `entry_file` against.

## Build

```sh
rustup target add wasm32-wasip2
cargo build --release --target wasm32-wasip2
```

`wasm32-wasip2` emits a component directly (rustc >= 1.82) — no
`cargo-component` and no `wasm-tools component new` step. `cargo component
build --release` also works if you prefer it; neither requires the other. Your
crate needs `crate-type = ["cdylib"]` (add `"rlib"` too if `tests/` links it)
and benefits from `opt-level = "s"`, `lto`, `panic = "abort"`, `strip` —
~200-400 KB.

A build driven by the CLI must scrub `CARGO_TARGET_DIR`, `RUSTFLAGS`,
`CARGO_ENCODED_RUSTFLAGS` and `CARGO_BUILD_RUSTFLAGS` from the environment: the
repo's `.cargo/config.toml` sets `split-debuginfo=unpacked`, which does not
exist on wasm32.

## Test

`cargo test` builds natively. `crate::host` then routes to
`raisin_sdk::testing::MockHost` instead of the WIT imports, so handlers, the
generated wrappers and `Transaction` all run their real code:

```rust
use raisin_sdk::testing::{with_mock, MockHost};

let mock = MockHost::new().expect("nodes_get", r#"["content","/x"]"#, Ok("null".into()));
let (result, mock) = with_mock(mock, || raisin_sdk::nodes::get("content", "/x"));
assert!(mock.unmet().is_empty());
```

An unscripted call is an `Err`, never a default answer.

## Layout

| file | hand-written? | what |
|---|---|---|
| `src/generated.rs` | **no** | typed wrappers for all 100+ `raisin.*` methods |
| `wit/raisin-function.wit` | **no** | copy of `crates/raisin-functions/wit/` |
| `src/host.rs` | yes | the gateway, split wasm / native |
| `src/wire.rs` | yes | reply decoders, incl. the `{"error":true}` envelope rule |
| `src/handler.rs` | yes | `Handler` + name routing |
| `src/error.rs`, `context.rs`, `log.rs`, `transaction.rs` | yes | sugar |
| `src/testing.rs` | yes | `MockHost` (native only) |
| `raisin-sdk-macros/` | yes | `#[handler]`, `export!` |

Regenerate the machine-owned files with `make gen-bindings` from the repo root;
`cargo test -p raisin-functions --lib` fails if they are stale.

**Do not run `cargo fmt` here** — it would reformat `src/generated.rs` and make
the freshness test permanently red. Format the hand-written files with
`rustfmt --edition 2021 --config skip_children=true <files>`; `skip_children` is
load-bearing, or rustfmt follows `mod generated;` out of `lib.rs` and reformats
it anyway. See `../../README.md`.

## Conventions

- **Every host error is an `Err`** (the Starlark convention). So is an `Ok` body
  carrying `{"error": true, "message": ...}` — a few host methods answer that
  way, and treating it as data would carry a failure forward as success.
- `Transaction` rolls back on drop unless committed.
- `raisin_sdk::context::Context::get()` reads the dedicated `context` import,
  not the gateway — it costs no round trip.
- `println!` / `eprintln!` reach the logs, but carry no level; prefer
  `raisin_sdk::log::*`.
- There is no socket and no native HTTP: egress is `raisin_sdk::http::*` only,
  and the host links neither `wasi:sockets` nor `wasi:http`.
