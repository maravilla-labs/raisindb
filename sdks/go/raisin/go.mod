module github.com/maravilla-labs/raisindb/sdks/go/raisin

// TinyGo >= 0.34 is the supported guest toolchain (`-target=wasip2`).
// The SDK itself has no third-party dependencies, so `go test ./...` runs
// natively against the mock host with a stock Go toolchain.
go 1.22
