// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

// Package raisintest is the native test harness for the Go guest SDK: a mock
// RaisinDB host you install for the duration of a test, so handlers can be
// exercised with `go test ./...` and no server, no wasm and no toolchain.
package raisintest

import (
	"encoding/json"
	"fmt"
	"strings"
	"sync"

	"github.com/maravilla-labs/raisindb/sdks/go/raisin"
)

// Call is one recorded host call.
type Call struct {
	// Method is the registry internal name, e.g. "nodes_getChildren".
	Method string
	// Args is the JSON array of positional arguments as it went over the wire.
	Args string
}

// LogLine is one recorded log call.
type LogLine struct {
	// Level is the level the guest logged at.
	Level raisin.LogLevel
	// Message is the formatted line.
	Message string
}

type expectation struct {
	method    string
	args      string
	anyArgs   bool
	result    string
	failure   string
	exhausted bool
}

// Mock is a scripted RaisinDB host. An unexpected call is an error, never a
// zero value: a handler that calls something the test did not script must
// fail, not silently read empty data.
type Mock struct {
	mu       sync.Mutex
	expects  []*expectation
	calls    []Call
	logs     []LogLine
	context  string
	abi      string
	strict   bool
	fallback string
}

// New returns a Mock with an empty context and the current host ABI version.
func New() *Mock {
	return &Mock{context: "{}", abi: "0.1.0", strict: true}
}

// Expect scripts one call. An empty args string matches any arguments;
// otherwise it is compared to the JSON array as encoded by the SDK. The result
// is the raw JSON payload the host would return (`"true"` for a void method,
// `"null"` for an absent optional).
func (m *Mock) Expect(method string, args string, result string) *Mock {
	m.mu.Lock()
	defer m.mu.Unlock()
	m.expects = append(m.expects, &expectation{
		method: method, args: args, anyArgs: args == "", result: result,
	})
	return m
}

// ExpectError scripts one call that fails with the given host message.
func (m *Mock) ExpectError(method string, args string, message string) *Mock {
	m.mu.Lock()
	defer m.mu.Unlock()
	m.expects = append(m.expects, &expectation{
		method: method, args: args, anyArgs: args == "", failure: message,
	})
	return m
}

// SetContext sets the JSON returned by raisin.Context.
func (m *Mock) SetContext(ctx string) *Mock {
	m.mu.Lock()
	defer m.mu.Unlock()
	m.context = ctx
	return m
}

// SetABIVersion sets the version returned by raisin.ABIVersion.
func (m *Mock) SetABIVersion(v string) *Mock {
	m.mu.Lock()
	defer m.mu.Unlock()
	m.abi = v
	return m
}

// AllowUnexpected makes unscripted calls return the given JSON payload instead
// of failing. Use it for a handler that logs or reads context incidentally.
func (m *Mock) AllowUnexpected(payload string) *Mock {
	m.mu.Lock()
	defer m.mu.Unlock()
	m.strict = false
	m.fallback = payload
	return m
}

// Install makes this mock the process-wide host and returns a restore function.
//
//	defer raisintest.New().Expect("nodes_get", "", `{"id":"n1"}`).Install()()
func (m *Mock) Install() (restore func()) {
	return raisin.SetHost(m)
}

// Calls returns every host call in order.
func (m *Mock) Calls() []Call {
	m.mu.Lock()
	defer m.mu.Unlock()
	return append([]Call(nil), m.calls...)
}

// Logs returns every log line in order.
func (m *Mock) Logs() []LogLine {
	m.mu.Lock()
	defer m.mu.Unlock()
	return append([]LogLine(nil), m.logs...)
}

// Unmet returns the scripted calls that never happened, for an end-of-test
// assertion.
func (m *Mock) Unmet() []string {
	m.mu.Lock()
	defer m.mu.Unlock()
	var out []string
	for _, e := range m.expects {
		if !e.exhausted {
			out = append(out, e.method)
		}
	}
	return out
}

// Call implements raisin.Host.
func (m *Mock) Call(method string, args string) (string, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	m.calls = append(m.calls, Call{Method: method, Args: args})
	for _, e := range m.expects {
		if e.exhausted || e.method != method {
			continue
		}
		if !e.anyArgs && e.args != args {
			continue
		}
		e.exhausted = true
		if e.failure != "" {
			return "", fmt.Errorf("%s", e.failure)
		}
		return e.result, nil
	}
	if !m.strict {
		return m.fallback, nil
	}
	return "", fmt.Errorf("raisintest: unexpected call %s(%s); scripted: %s", method, args, m.scripted())
}

// scripted renders the still-unmet expectations for an error message.
func (m *Mock) scripted() string {
	var names []string
	for _, e := range m.expects {
		if !e.exhausted {
			names = append(names, e.method)
		}
	}
	if len(names) == 0 {
		return "(none)"
	}
	return strings.Join(names, ", ")
}

// Log implements raisin.Host.
func (m *Mock) Log(level raisin.LogLevel, message string) {
	m.mu.Lock()
	defer m.mu.Unlock()
	m.logs = append(m.logs, LogLine{Level: level, Message: message})
}

// Context implements raisin.Host.
func (m *Mock) Context() string {
	m.mu.Lock()
	defer m.mu.Unlock()
	return m.context
}

// ABIVersion implements raisin.Host.
func (m *Mock) ABIVersion() string {
	m.mu.Lock()
	defer m.mu.Unlock()
	return m.abi
}

// Invoke routes an invocation through the SDK exactly as the host would,
// including the unknown-handler error. It is the one call a handler test needs.
func Invoke(name string, input any) (json.RawMessage, error) {
	encoded, err := json.Marshal(input)
	if err != nil {
		return nil, err
	}
	out, err := raisin.Dispatch(name, string(encoded))
	if err != nil {
		return nil, err
	}
	return json.RawMessage(out), nil
}
