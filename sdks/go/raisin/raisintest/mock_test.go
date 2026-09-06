// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

package raisintest_test

import (
	"encoding/json"
	"strings"
	"testing"

	"github.com/maravilla-labs/raisindb/sdks/go/raisin"
	"github.com/maravilla-labs/raisindb/sdks/go/raisin/raisintest"
)

func init() {
	raisin.HandleDefault(func(input json.RawMessage) (any, error) {
		var in struct {
			Name string `json:"name"`
		}
		if err := json.Unmarshal(input, &in); err != nil {
			return nil, err
		}
		people, err := raisin.Nodes.GetChildren("content", "/people", nil)
		if err != nil {
			return nil, err
		}
		var rows []any
		if err := json.Unmarshal(people, &rows); err != nil {
			return nil, err
		}
		raisin.Info("greeting %s", in.Name)
		return map[string]any{"greeting": "Hello, " + in.Name, "people": len(rows)}, nil
	})
}

func TestHandlerRunsAgainstTheMockHost(t *testing.T) {
	mock := raisintest.New().
		SetContext(`{"tenant_id":"acme"}`).
		Expect("nodes_getChildren", `["content","/people",null]`, `[{"id":"a"},{"id":"b"}]`)
	defer mock.Install()()

	out, err := raisintest.Invoke("default", map[string]string{"name": "Ada"})
	if err != nil {
		t.Fatalf("invoke: %v", err)
	}
	if string(out) != `{"greeting":"Hello, Ada","people":2}` {
		t.Fatalf("unexpected output %s", out)
	}
	if len(mock.Unmet()) != 0 {
		t.Fatalf("unmet expectations: %v", mock.Unmet())
	}
	if logs := mock.Logs(); len(logs) != 1 || logs[0].Message != "greeting Ada" {
		t.Fatalf("unexpected logs %v", logs)
	}
	if calls := mock.Calls(); len(calls) != 1 || calls[0].Method != "nodes_getChildren" {
		t.Fatalf("unexpected calls %v", calls)
	}
}

func TestUnexpectedCallIsAnError(t *testing.T) {
	mock := raisintest.New()
	defer mock.Install()()

	_, err := raisintest.Invoke("default", map[string]string{"name": "Ada"})
	if err == nil || !strings.Contains(err.Error(), "unexpected call nodes_getChildren") {
		t.Fatalf("expected an unexpected-call error, got %v", err)
	}
}

func TestInvokeSurfacesTheUnknownHandlerError(t *testing.T) {
	defer raisintest.New().Install()()

	_, err := raisintest.Invoke("nope", nil)
	if err == nil || !strings.Contains(err.Error(), "registered: default") {
		t.Fatalf("expected the registered set in the error, got %v", err)
	}
}

func TestScriptedFailureReachesTheHandler(t *testing.T) {
	mock := raisintest.New().
		ExpectError("nodes_getChildren", "", "workspace not found")
	defer mock.Install()()

	_, err := raisintest.Invoke("default", map[string]string{"name": "Ada"})
	if err == nil || !strings.Contains(err.Error(), "workspace not found") {
		t.Fatalf("expected the host failure, got %v", err)
	}
}
