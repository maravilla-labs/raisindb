// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//go:build wasip2

package raisin

import "github.com/maravilla-labs/raisindb/sdks/go/raisin/internal/wit"

// componentHost is the real host: the four `raisin:function/host` imports.
type componentHost struct{}

// Call implements Host over the `host.call` import.
func (componentHost) Call(method string, args string) (string, error) {
	return wit.Call(method, args)
}

// Log implements Host over the `host.log` import.
func (componentHost) Log(level LogLevel, message string) {
	wit.Log(wit.LogLevel(level), message)
}

// Context implements Host over the `host.context` import.
func (componentHost) Context() string { return wit.Context() }

// ABIVersion implements Host over the `host.abi-version` import.
func (componentHost) ABIVersion() string { return wit.ABIVersion() }

// init installs the component host and claims the single `handler(name, input)`
// export, routing it through Dispatch. Handlers register from their own init
// functions; Go runs every package init before the export can be called.
func init() {
	defaultHost = componentHost{}
	wit.SetHandler(Dispatch)
}
