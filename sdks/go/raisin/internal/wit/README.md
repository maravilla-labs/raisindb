# internal/wit — component-model bindings

This package is the only place the Go SDK touches the component model. Every
other file talks to the four functions `Call`, `Log`, `Context`, `ABIVersion`
and the export installer `SetHandler`.

## Current state

`wit_wasip2.go` is a **hand-written stub**. It compiles and has the right
shape, but it is not wired to the component model: host calls return
`ErrBindingsNotGenerated`. Nothing in this repository can generate it, because
`go`, `tinygo` and `wit-bindgen-go` are not installed on the build machine.

## Regenerating (do this before shipping a component)

From `sdks/go/raisin`:

```sh
go run go.bytecodealliance.org/cmd/wit-bindgen-go@v0.6.2 generate \
    --world function --out internal/wit ./wit/raisin-function.wit
```

The generated tree is committed so SDK users never need `wit-bindgen-go`. It
lands as `internal/wit/raisin/function/host/host.wit.go` (imports) plus an
exports package, using `cm.Result`/`cm.Option` wrappers from
`go.bytecodealliance.org/cm`.

Then delete `wit_wasip2.go` and add `adapt_wasip2.go` in its place, keeping
**exactly** the signatures the SDK depends on:

```go
type LogLevel uint8
const (LogLevelDebug LogLevel = iota; LogLevelInfo; LogLevelWarn; LogLevelError)

func Call(method string, args string) (string, error)
func Log(level LogLevel, message string)
func Context() string
func ABIVersion() string
func SetHandler(fn func(name string, input string) (string, error))
```

`SetHandler` assigns the generated `exports.Handler` variable a body that
converts the SDK's `(string, error)` into the WIT `result<string, string>`:
an error becomes `cm.Err[cm.Result[string, string, string]](err.Error())`.

`wit/raisin-function.wit` is a byte-identical copy of
`crates/raisin-functions/wit/raisin-function.wit`, written by
`make gen-bindings` and guarded by the `sdk_wit_copies_match_source` test.
Never edit it here.

## Build

```sh
tinygo build -target=wasip2 -o main.wasm --wit-package ./wit --wit-world function .
```

TinyGo >= 0.34 is required for `-target=wasip2`; the flag spelling above is
what that release documents and has **not** been verified on this machine.
