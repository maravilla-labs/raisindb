// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

package raisin

import "sync"

// LogLevel mirrors the WIT `log-level` enum.
type LogLevel uint8

// The log levels understood by the host.
const (
	LevelDebug LogLevel = iota
	LevelInfo
	LevelWarn
	LevelError
)

// String returns the lowercase level name used by the host log surface.
func (l LogLevel) String() string {
	switch l {
	case LevelDebug:
		return "debug"
	case LevelInfo:
		return "info"
	case LevelWarn:
		return "warn"
	case LevelError:
		return "error"
	default:
		return "info"
	}
}

// Host is the RaisinDB host surface, one method per WIT import.
//
// There is exactly one implementation per build target: the component-model
// bindings under `wasip2`/`tinygo`, and whatever a test installs natively.
// Guest code never talks to a Host directly — it calls the generated
// namespaces, which funnel through the call helpers in call.go.
type Host interface {
	// Call invokes a RaisinDB API method by registry internal name with a
	// JSON array of positional arguments, returning the raw JSON payload.
	Call(method string, args string) (string, error)
	// Log emits a structured log line into the execution result.
	Log(level LogLevel, message string)
	// Context returns the execution-context JSON, byte-identical to
	// `raisin.context.get()` in JavaScript and Starlark.
	Context() string
	// ABIVersion returns the host ABI semver, e.g. "0.1.0".
	ABIVersion() string
}

var (
	hostMu      sync.RWMutex
	installed   Host
	defaultHost Host
)

// SetHost installs a Host, returning a function that restores the previous
// one. Tests use it through raisintest; guest code never calls it.
func SetHost(h Host) (restore func()) {
	hostMu.Lock()
	prev := installed
	installed = h
	hostMu.Unlock()
	return func() {
		hostMu.Lock()
		installed = prev
		hostMu.Unlock()
	}
}

// currentHost returns the installed host, falling back to the build target's
// default (the component bindings on wasm, a failing stub natively).
func currentHost() Host {
	hostMu.RLock()
	h := installed
	hostMu.RUnlock()
	if h != nil {
		return h
	}
	return defaultHost
}

// ABIVersion reports the host ABI version this component is running against.
func ABIVersion() string {
	return currentHost().ABIVersion()
}
