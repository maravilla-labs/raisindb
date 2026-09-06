// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

package raisin

import (
	"errors"
	"strings"
	"testing"
)

// stubHost is a minimal Host for the in-package call tests. The richer mock
// lives in raisintest, which cannot be imported here (it imports this package).
type stubHost struct {
	method  string
	args    string
	payload string
	err     error
	logs    []string
}

func (s *stubHost) Call(method string, args string) (string, error) {
	s.method, s.args = method, args
	return s.payload, s.err
}

func (s *stubHost) Log(level LogLevel, message string) {
	s.logs = append(s.logs, level.String()+":"+message)
}

func (s *stubHost) Context() string    { return `{"tenant_id":"acme","repo_id":"main"}` }
func (s *stubHost) ABIVersion() string { return "0.1.0" }

func TestCallEncodesPositionalArguments(t *testing.T) {
	limit := uint32(10)
	h := &stubHost{payload: `[{"id":"n1"}]`}
	defer SetHost(h)()

	got, err := Nodes.GetChildren("content", "/people", &limit)
	if err != nil {
		t.Fatalf("GetChildren: %v", err)
	}
	if h.method != "nodes_getChildren" {
		t.Fatalf("unexpected method %q", h.method)
	}
	if h.args != `["content","/people",10]` {
		t.Fatalf("unexpected args %q", h.args)
	}
	if string(got) != `[{"id":"n1"}]` {
		t.Fatalf("unexpected payload %s", got)
	}
}

func TestNilOptionalBecomesJSONNull(t *testing.T) {
	h := &stubHost{payload: `[]`}
	defer SetHost(h)()

	if _, err := Nodes.GetChildren("content", "/people", nil); err != nil {
		t.Fatalf("GetChildren: %v", err)
	}
	if h.args != `["content","/people",null]` {
		t.Fatalf("expected a null optional, got %q", h.args)
	}
}

func TestZeroArgumentCallSendsAnEmptyArray(t *testing.T) {
	h := &stubHost{payload: `[]`}
	defer SetHost(h)()

	if _, err := Ai.ListModels(); err != nil {
		t.Fatalf("ListModels: %v", err)
	}
	if h.args != "[]" {
		t.Fatalf("expected an empty argument array, got %q", h.args)
	}
}

func TestErrorEnvelopeIsAnError(t *testing.T) {
	h := &stubHost{payload: `{"error":true,"message":"node not found","code":"NOT_FOUND"}`}
	defer SetHost(h)()

	_, err := Nodes.Get("content", "/missing")
	if err == nil {
		t.Fatal("an {\"error\":true} envelope must be an error")
	}
	if !strings.Contains(err.Error(), "node not found") || !strings.Contains(err.Error(), "NOT_FOUND") {
		t.Fatalf("unexpected message %q", err.Error())
	}
	var he *HostError
	if !errors.As(err, &he) || he.Method != "nodes_get" {
		t.Fatalf("expected a HostError naming the method, got %#v", err)
	}
}

func TestHostErrorIsWrapped(t *testing.T) {
	h := &stubHost{err: errors.New("Unknown raisin API method: nope")}
	defer SetHost(h)()

	if _, err := Nodes.Get("content", "/x"); err == nil ||
		!strings.Contains(err.Error(), "Unknown raisin API method") {
		t.Fatalf("expected the host message to survive, got %v", err)
	}
}

func TestScalarDecoding(t *testing.T) {
	h := &stubHost{payload: "true"}
	restore := SetHost(h)
	ok, err := Locks.Release("seat-1", 7)
	restore()
	if err != nil || !ok {
		t.Fatalf("expected true, got %v %v", ok, err)
	}

	h = &stubHost{payload: "42"}
	restore = SetHost(h)
	n, err := Date.AddDays(0, 1)
	restore()
	if err != nil || n != 42 {
		t.Fatalf("expected 42, got %v %v", n, err)
	}

	h = &stubHost{payload: `"deadbeef"`}
	restore = SetHost(h)
	digest, err := Crypto.Hash("x", nil)
	restore()
	if err != nil || digest != "deadbeef" {
		t.Fatalf("expected the decoded string, got %q %v", digest, err)
	}

	// A void method's wire value is the literal `true` and is ignored.
	h = &stubHost{payload: "true"}
	restore = SetHost(h)
	err = Events.Emit("thing.happened", map[string]any{"id": 1})
	restore()
	if err != nil {
		t.Fatalf("void call: %v", err)
	}
}

func TestIntoDecodesTypedResults(t *testing.T) {
	h := &stubHost{payload: `{"name":"Ada"}`}
	defer SetHost(h)()

	type person struct {
		Name string `json:"name"`
	}
	p, err := Into[person](Nodes.Get("content", "/people/ada"))
	if err != nil {
		t.Fatalf("Into: %v", err)
	}
	if p.Name != "Ada" {
		t.Fatalf("unexpected decode %#v", p)
	}
}

func TestContextAndLogging(t *testing.T) {
	h := &stubHost{}
	defer SetHost(h)()

	ctx, err := Context()
	if err != nil {
		t.Fatalf("Context: %v", err)
	}
	if ctx.Tenant != "acme" || ctx.Repo != "main" {
		t.Fatalf("unexpected context %#v", ctx)
	}
	if string(ContextJSON()) != h.Context() {
		t.Fatal("ContextJSON must be the raw host string")
	}
	Warn("careful: %d", 7)
	if len(h.logs) != 1 || h.logs[0] != "warn:careful: 7" {
		t.Fatalf("unexpected logs %v", h.logs)
	}
	if ABIVersion() != "0.1.0" {
		t.Fatalf("unexpected abi %q", ABIVersion())
	}
}

func TestAbsentHostFailsByName(t *testing.T) {
	_, err := Nodes.Get("content", "/x")
	if err == nil || !strings.Contains(err.Error(), "no host installed") {
		t.Fatalf("expected the absent-host error, got %v", err)
	}
}
