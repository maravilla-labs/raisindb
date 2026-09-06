// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//go:build wasip2 || tinygo

// Package wit is the component-model seam of the Go guest SDK: the four
// `raisin:function/host` imports and the single `handler(name, input)` export.
//
// TODO(B5): this file is a HAND-WRITTEN STUB. It has the right shape and
// compiles, but it does not talk to the component model, so a component built
// against it answers every host call with ErrBindingsNotGenerated. Replace it
// with the real bindings — see internal/wit/README.md — by running, from
// sdks/go/raisin:
//
//	go run go.bytecodealliance.org/cmd/wit-bindgen-go@v0.6.2 generate \
//	    --world function --out internal/wit ./wit/raisin-function.wit
//
// and then deleting this file. The generated package exposes the imports as
// `host.Call/Log/Context/ABIVersion` returning `cm.Result` values and the
// export as an assignable `exports.Handler` var; adapt.go (also to write) is
// the thin translation to the signatures below, which is all the rest of the
// SDK depends on.
package wit

import "errors"

// LogLevel mirrors the WIT `log-level` enum, in declaration order.
type LogLevel uint8

// The WIT log levels.
const (
	LogLevelDebug LogLevel = iota
	LogLevelInfo
	LogLevelWarn
	LogLevelError
)

// ErrBindingsNotGenerated is returned by every host call while this stub is
// in place, so a component built without regenerating fails loudly and by
// name instead of silently returning empty results.
var ErrBindingsNotGenerated = errors.New(
	"raisin: WIT bindings not generated - run `wit-bindgen-go generate --world function --out internal/wit ./wit/raisin-function.wit`")

// handler holds the routed export body installed by the SDK.
var handler func(name string, input string) (string, error)

// SetHandler installs the routed body of the `handler(name, input)` export.
// The real bindings assign it to the generated export variable; the stub only
// stores it, so Invoke can exercise it in a non-component build.
func SetHandler(fn func(name string, input string) (string, error)) {
	handler = fn
}

// Invoke calls the installed export body. The real bindings never need this —
// the host calls the export directly — but it keeps the stub's registration
// observable rather than dead.
func Invoke(name string, input string) (string, error) {
	if handler == nil {
		return "", errors.New("raisin: no handler installed")
	}
	return handler(name, input)
}

// Call is the `host.call(method, args)` import.
func Call(method string, args string) (string, error) {
	_, _ = method, args
	return "", ErrBindingsNotGenerated
}

// Log is the `host.log(level, message)` import.
func Log(level LogLevel, message string) {
	_, _ = level, message
}

// Context is the `host.context()` import.
func Context() string {
	return "{}"
}

// ABIVersion is the `host.abi-version()` import.
func ABIVersion() string {
	return ""
}
