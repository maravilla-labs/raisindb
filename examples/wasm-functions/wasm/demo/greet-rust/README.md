# greet-rust

The Rust half of the example, built on [`raisin-sdk`](../../../../../sdks/rust/raisin-sdk).

Two handlers in one crate — `default` (`greet`) and `shout` — registered by
name and listed in `raisin_sdk::export!`. That list is what makes
`content/functions/lib/demo/greet-rust-shout/.node.yaml` legal: its
`entry_file` is `../greet-rust/main.wasm:shout`, pointing at THIS artifact.

```sh
cargo test                                      # native, against MockHost
cargo build --release --target wasm32-wasip2    # -> target/wasm32-wasip2/release/greet_rust.wasm
```

`wasm32-wasip2` emits a component directly (rustc >= 1.82); `cargo-component`
is optional. `tests/server.json` is the against-a-server suite
(`raisindb function test --server`).
