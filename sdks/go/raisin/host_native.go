// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//go:build !wasip2

package raisin

import "errors"

// errNoHost is what an un-mocked host call fails with in a native build. It is
// deliberately an error rather than a panic: a test that forgot to install a
// mock should fail on the assertion it was writing, not on a stack trace.
var errNoHost = errors.New(
	"raisin: no host installed - this build is not a WebAssembly component; install a mock with raisintest.New().Install()")

// absentHost is the native default: every call fails, logs are dropped and the
// execution context is empty.
type absentHost struct{}

// Call implements Host by failing.
func (absentHost) Call(method string, args string) (string, error) {
	_, _ = method, args
	return "", errNoHost
}

// Log implements Host by discarding the line.
func (absentHost) Log(level LogLevel, message string) { _, _ = level, message }

// Context implements Host with an empty context.
func (absentHost) Context() string { return "{}" }

// ABIVersion implements Host with an empty version.
func (absentHost) ABIVersion() string { return "" }

// init installs the native default host.
func init() {
	defaultHost = absentHost{}
}
