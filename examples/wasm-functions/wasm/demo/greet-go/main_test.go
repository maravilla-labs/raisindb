// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

package main

import (
	"strings"
	"testing"

	"github.com/maravilla-labs/raisindb/sdks/go/raisin/raisintest"
)

func TestGreet(t *testing.T) {
	mock := raisintest.New().
		Expect("nodes_getChildren", "", `[{"id":"a"},{"id":"b"}]`)
	defer mock.Install()()

	out, err := raisintest.Invoke("default", map[string]string{"name": "Ada"})
	if err != nil {
		t.Fatalf("invoke: %v", err)
	}
	if !strings.Contains(string(out), `"greeting":"Hello, Ada!"`) ||
		!strings.Contains(string(out), `"people":2`) {
		t.Fatalf("unexpected output %s", out)
	}
}

func TestShoutIsTheSameArtifact(t *testing.T) {
	mock := raisintest.New().Expect("nodes_getChildren", "", `[]`)
	defer mock.Install()()

	out, err := raisintest.Invoke("shout", map[string]string{"name": "Ada"})
	if err != nil {
		t.Fatalf("invoke: %v", err)
	}
	if !strings.Contains(string(out), `"greeting":"HELLO, ADA!"`) {
		t.Fatalf("unexpected output %s", out)
	}
}

func TestMissingNameIsAnError(t *testing.T) {
	defer raisintest.New().Install()()

	if _, err := raisintest.Invoke("default", map[string]string{}); err == nil {
		t.Fatal("expected input.name to be required")
	}
}
