# raisin — Go guest SDK for RaisinDB WebAssembly functions

Build a RaisinDB function as a WebAssembly component in Go (TinyGo,
`-target=wasip2`), and test it natively with `go test ./...`.

```go
package main

import (
	"encoding/json"

	"github.com/maravilla-labs/raisindb/sdks/go/raisin"
)

func init() {
	raisin.HandleDefault(greet)      // entry_file: main.wasm
	raisin.Handle("shout", shout)    // entry_file: main.wasm:shout
}

func main() {}

func greet(input json.RawMessage) (any, error) {
	people, err := raisin.Nodes.GetChildren("content", "/people", nil)
	if err != nil {
		return nil, err
	}
	raisin.Info("greeting from go")
	return map[string]any{"people": string(people)}, nil
}
```

## Name-routed handlers

The component exports exactly one WIT function,
`handler(name, input) -> result<string, string>`. The SDK owns that export and
routes on `name` against the map `Handle` writes into, so:

- ONE artifact carries N handlers.
- N `raisin:Function` nodes can share one artifact
  (`entry_file: ../shared/main.wasm:on-order`).
- The host **never** validates a handler name. An unknown name comes back as
  an error naming everything the guest registered — that error is the only
  diagnosis a typo'd `entry_file` gets, so keep it intact.
- A bare `entry_file: main.wasm` selects `"default"`; `HandleDefault` is the
  sugar for it.

Registering the same name twice panics rather than silently shadowing.

## Layout

| file | hand-written? | content |
|---|---|---|
| `generated.go` | **no** — `make gen-bindings` | the whole `raisin.*` surface, one method per registry descriptor |
| `wit/raisin-function.wit` | **no** — `make gen-bindings` | byte-identical copy of `crates/raisin-functions/wit/` |
| `host.go` | yes | the `Host` interface, `SetHost` |
| `call.go` | yes | `callJSON/Bool/Int64/String/Void`, the `{"error":true}` envelope rule, `Into[T]` |
| `handler.go` | yes | `Handle` / `HandleDefault` / `Dispatch` |
| `context.go`, `log.go` | yes | `raisin.Context()`, `raisin.Info(...)` — the two registry methods excluded from generation |
| `host_wasip2.go` / `host_native.go` | yes | the component host vs. the native "no host installed" default |
| `internal/wit/` | yes, **STUB** | the component-model seam — see its README |
| `raisintest/` | yes | mock host + `Invoke(name, input)` |

Both generated files are checked by `cargo test -p raisin-functions --lib`;
edit `crates/raisin-functions/src/runtime/bindings/` and re-run
`make gen-bindings` instead of touching them.

## Conventions

- **Every host error is a Go `error`** (the Starlark convention), including an
  `{"error": true, "message": ...}` payload returned inside an Ok — it becomes
  a `*HostError` naming the registry method.
- Optional arguments are pointers; `nil` goes over the wire as JSON `null`.
- A void method's wire value is the literal `true` and is ignored.
- `Into[T](raisin.Nodes.Get(...))` decodes a result in one call.

## Testing

```sh
go test ./...
```

`raisintest.New()` is a scripted host: `Expect(method, args, result)`,
`ExpectError(...)`, `SetContext(...)`, `Logs()`, `Calls()`, `Unmet()`. An
unscripted call is an error, never a zero value — a handler must not silently
read empty data. `raisintest.Invoke(name, input)` routes through `Dispatch`
exactly as the host would, unknown-handler error included.

## Building a component

```sh
tinygo build -target=wasip2 -o main.wasm \
    --wit-package ./wit --wit-world function .
```

**Not yet verified**: `go`, `tinygo` and `wit-bindgen-go` are not installed on
the machine this SDK was written on. `internal/wit` is a hand-written stub with
the right shape; regenerate it (see `internal/wit/README.md`) before shipping a
component, or every host call fails with `ErrBindingsNotGenerated`.
