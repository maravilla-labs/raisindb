# WebAssembly functions example

One package, three languages, and the point it makes: **a wasm artifact is
shared, not duplicated per function.**

```
manifest.yaml
.rapignore                                    wasm/  — source is not package content
content/functions/lib/demo/
    greet-rust/       .node.yaml  main.wasm   entry_file: main.wasm                    -> handler "default"
    greet-rust-shout/ .node.yaml              entry_file: ../greet-rust/main.wasm:shout -> handler "shout"
    greet-go/         .node.yaml  main.wasm
    greet-ts/         .node.yaml  main.wasm
wasm/demo/
    greet-rust/       Cargo.toml raisin.build.yaml src/ tests/     (does NOT ship)
    greet-go/         go.mod     raisin.build.yaml main.go
    greet-ts/         package.json raisin.build.yaml src/
```

All three greet by name, count the children of `/people` in the `content`
workspace through the host gateway, and log one line — identical behaviour,
so `smoke.mjs` can assert identical outputs.

## One artifact, N handlers

`greet-rust` and `greet-rust-shout` are TWO `raisin:Function` nodes backed by
ONE uploaded `main.wasm`. The WIT export is fixed — `handler(name, input)` —
and the handler *name* comes from the `entry_file` suffix, so a bare
`main.wasm` selects `default` and `../greet-rust/main.wasm:shout` selects
`shout` out of the same bytes. A parent-relative `entry_file` must resolve
inside the functions workspace; the server refuses one that escapes it, and an
unregistered name comes back as the guest's own error naming what it did
register.

## Build (Rust)

```sh
rustup target add wasm32-wasip2
cd wasm/demo/greet-rust
cargo test                                     # native: no server, no wasm runtime
cargo build --release --target wasm32-wasip2
cp target/wasm32-wasip2/release/greet_rust.wasm \
   ../../../content/functions/lib/demo/greet-rust/main.wasm
```

`raisindb function build` does the last two steps and lists every Function node
whose `entry_file` resolves to the artifact it just wrote — so a build that
backs two handlers says so. See each project's `raisin.build.yaml` for the Go
and TypeScript commands.

## Build (all three)

```sh
make wasm-sdks-build                # every project, needs cargo + tinygo + node
make wasm-sdks-build LANGS="rust"   # just one
```

Both go through `raisindb function build`, so the build → copy → "which Function
nodes does this artifact back?" rule has one implementation. Only
`greet-rust/main.wasm` is committed (~180 KB); the TinyGo (~2 MB) and jco
(~12 MB) artifacts are build output and gitignored.

## Deploy and invoke

```sh
raisindb deploy examples/wasm-functions --install
raisindb function run content/functions/lib/demo/greet-rust       --input '{"name":"Ada"}'
raisindb function run content/functions/lib/demo/greet-rust-shout --input '{"name":"Ada"}'
```

Expected: `Hello, Ada!` and `HELLO, ADA!`, the same `people` count from both,
and one log line each.

## Smoke test

`smoke.mjs` is that whole loop as one assertion: authenticate → `raisindb
deploy . --install` (the real CLI, not a reimplementation of it) → invoke every
Function node → assert the three languages answer identically and that
`greet-rust` / `greet-rust-shout` each ran their own handler out of the one
uploaded artifact.

```sh
make wasm-smoke                                   # from the repo root
node smoke.mjs --server http://localhost:8080 --repo wasmdemo
node smoke.mjs --skip-deploy                      # invoke an already-installed package
```

Credentials come from `RAISIN_USER` / `RAISIN_PASSWORD` (default the dev admin);
the server, repo and tenant from `--server` / `--repo` / `--tenant` or
`RAISINDB_SERVER` / `RAISINDB_REPO`. Node builtins only — no `npm install`.

An artifact that is not built is a hard failure, not a skip: `raisindb deploy`
validates that every `language: wasm` node's `entry_file` target exists, so the
deploy would fail anyway and the useful message is the one naming the build
command.

## Tests without a server

The guest SDKs run natively against a mock host, so the handler logic in each
project is testable with no wasm toolchain and no RaisinDB running:

```sh
make wasm-sdks-test              # the three SDK suites (Rust, Go, TypeScript)
cd wasm/demo/greet-rust && cargo test
cd wasm/demo/greet-go   && go test ./...
cd wasm/demo/greet-ts   && npm test
```

CI runs `make wasm-sdks-test` with `WASM_SDKS_STRICT=1`, which turns a missing
toolchain from a skip into a failure — a suite that quietly stops running is
the failure that job exists to catch.
