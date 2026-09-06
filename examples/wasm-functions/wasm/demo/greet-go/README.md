# greet-go — a RaisinDB function as a WebAssembly component

Source for `content/functions/lib/demo/greet-go/main.wasm`. Nothing in this
directory ships: `.rapignore` keeps `wasm/` out of the package, so `raisindb
sync` never uploads `go.mod` as an asset.

## Handlers

One artifact, two handlers, routed inside the single `handler(name, input)`
export:

| handler | `entry_file` |
|---|---|
| `default` | `main.wasm` |
| `shout` | `main.wasm:shout` |

A second Function node can point at this same artifact from its own directory
with `entry_file: ../greet-go/main.wasm:shout` — that is the
one-artifact-N-functions path.

## Test (no server, no wasm toolchain)

```sh
go test ./...
```

The handlers run natively against `raisintest`'s mock host.

## Build

```sh
raisindb function build examples/wasm-functions/wasm/demo/greet-go
```

or, by hand:

```sh
tinygo build -target=wasip2 -o main.wasm \
    --wit-package ../../../../../sdks/go/raisin/wit --wit-world function .
```

TinyGo >= 0.34 is required. **Not yet verified on a machine with TinyGo
installed**, and the SDK's `internal/wit` bindings are still a stub — see
`sdks/go/raisin/internal/wit/README.md`.

## Run against a local server

```sh
raisindb function run examples/wasm-functions/wasm/demo/greet-go \
    --input '{"name":"Ada"}'
raisindb function test examples/wasm-functions/wasm/demo/greet-go --server
```
