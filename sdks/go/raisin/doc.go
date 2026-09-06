// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

// Package raisin is the Go guest SDK for RaisinDB WebAssembly functions.
//
// A function component exports exactly one WIT function,
// `handler(name, input) -> result<string, string>`. The SDK owns that export
// and routes on `name`, so ONE artifact can carry many handlers and many
// `raisin:Function` nodes can share it (`entry_file: ../shared/main.wasm:on-order`).
//
// Register handlers by name from `init` or `main`:
//
//	func init() {
//	    raisin.HandleDefault(greet)          // entry_file: main.wasm
//	    raisin.Handle("shout", shout)        // entry_file: main.wasm:shout
//	}
//
// The host surface (`raisin.Nodes.Get(...)`, `raisin.Sql.Query(...)`, ...) is
// machine-generated from the RaisinDB descriptor registry into generated.go;
// every call funnels through the single `host.call(method, args)` WIT import.
//
// Build with TinyGo:
//
//	tinygo build -target=wasip2 -o main.wasm \
//	    --wit-package ./wit --wit-world function .
package raisin
