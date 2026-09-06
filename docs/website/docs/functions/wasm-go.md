---
title: WebAssembly — Go
description: Write a RaisinDB function in Go, build it with TinyGo for wasip2, and test it with go test.
---

# WebAssembly functions in Go

The Go guest SDK is `github.com/maravilla-labs/raisindb/sdks/go/raisin`
(`sdks/go/raisin` in the repository). Handlers register in `init()`, the SDK
owns the single WIT export and routes on the name, and `go test ./...` runs
everything natively against a mock host.

```bash
raisindb create function greet --lang go
```

## A function

```go
package main

import (
	"encoding/json"
	"fmt"
	"strings"

	"github.com/maravilla-labs/raisindb/sdks/go/raisin"
)

type input struct{ Name string `json:"name"` }

func init() {
	raisin.HandleDefault(greet)   // entry_file: main.wasm
	raisin.Handle("shout", shout) // entry_file: main.wasm:shout
}

// Required by Go, never called: the host calls the component export.
func main() {}

func greet(raw json.RawMessage) (any, error) {
	var in input
	if err := json.Unmarshal(raw, &in); err != nil {
		return nil, err
	}
	children, err := raisin.Nodes.GetChildren("content", "/people", nil)
	if err != nil {
		return nil, err
	}
	raisin.Info("greeting %s", in.Name)
	return map[string]any{"greeting": fmt.Sprintf("Hello, %s!", in.Name), "people": children}, nil
}

func shout(raw json.RawMessage) (any, error) {
	var in input
	if err := json.Unmarshal(raw, &in); err != nil {
		return nil, err
	}
	return map[string]any{"greeting": strings.ToUpper("Hello, " + in.Name + "!")}, nil
}
```

`HandlerFunc` is `func(json.RawMessage) (any, error)`. `raisin.Handle` panics on
a duplicate name rather than shadowing — a silently overwritten handler would be
unreachable with no error anywhere. `raisin.RegisteredHandlers()` returns the
sorted set, and an unknown name comes back as
`unknown handler "nope"; registered: default, shout` — the only diagnosis a
typo'd `entry_file` gets, so keep it intact.

## Build

```bash
tinygo build -target=wasip2 -o main.wasm \
    --wit-package <sdk>/wit --wit-world function .
```

`raisindb function build` runs the `command:` from `raisin.build.yaml` and
copies the artifact into the Function node. Components land around 1–3 MB.

:::caution Unverified toolchain
The Go SDK was written on a machine with no Go toolchain: it has never been
compiled, and `internal/wit` — the component-model seam — is a **stub** whose
host calls all return `ErrBindingsNotGenerated`. Regenerate it with
`wit-bindgen-go` (see `sdks/go/raisin/internal/wit/README.md`) before shipping a
Go component. The Rust and TypeScript lanes are built and exercised; this one is
not yet.
:::

## Testing, natively

```bash
go test ./...
```

```go
mock := raisintest.New().
	Expect("nodes_getChildren", `["content","/people",null]`, `[]`).
	SetContext(map[string]any{"tenant_id": "t1"})
mock.Install()

out, err := raisintest.Invoke("default", `{"name":"Ada"}`)
```

An unscripted call is an error, never a zero value — a handler must not silently
read empty data. `raisintest.Invoke` routes through the same `Dispatch` the host
uses, unknown-handler error included.

## Conventions

- **Every host error is a Go `error`**, including an `{"error": true, …}`
  payload returned inside an Ok: it becomes a `*HostError` naming the registry
  method, recoverable with `errors.As`.
- Optional arguments are pointers; `nil` goes over the wire as JSON `null`.
- A void method's wire value is the literal `true`, and is ignored.
- `Into[T](raisin.Nodes.Get(...))` decodes a result in one call.
- `raisin.Context()` / `raisin.ContextInto[T]()` read the dedicated context
  import.

`generated.go` and `wit/raisin-function.wit` are machine-owned
(`make gen-bindings`); edit the server's binding registry instead.
