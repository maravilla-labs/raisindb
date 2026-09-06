module github.com/maravilla-labs/raisindb/examples/wasm-functions/greet-go

go 1.22

require github.com/maravilla-labs/raisindb/sdks/go/raisin v0.0.0

// In-repo example: build against the SDK in this checkout rather than a
// published module.
replace github.com/maravilla-labs/raisindb/sdks/go/raisin => ../../../../../sdks/go/raisin
