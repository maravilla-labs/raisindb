// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

package raisin

import (
	"encoding/json"
	"strings"
	"testing"
)

type greeting struct {
	Name string `json:"name"`
}

func TestDispatchRoutesByName(t *testing.T) {
	resetHandlers()
	defer resetHandlers()

	HandleDefault(func(input json.RawMessage) (any, error) {
		var in greeting
		if err := json.Unmarshal(input, &in); err != nil {
			return nil, err
		}
		return map[string]string{"greeting": "Hello, " + in.Name}, nil
	})
	Handle("shout", func(input json.RawMessage) (any, error) {
		var in greeting
		if err := json.Unmarshal(input, &in); err != nil {
			return nil, err
		}
		return map[string]string{"greeting": strings.ToUpper(in.Name) + "!"}, nil
	})

	out, err := Dispatch("default", `{"name":"Ada"}`)
	if err != nil {
		t.Fatalf("default handler: %v", err)
	}
	if out != `{"greeting":"Hello, Ada"}` {
		t.Fatalf("unexpected default output: %s", out)
	}

	out, err = Dispatch("shout", `{"name":"Ada"}`)
	if err != nil {
		t.Fatalf("shout handler: %v", err)
	}
	if out != `{"greeting":"ADA!"}` {
		t.Fatalf("unexpected shout output: %s", out)
	}
}

func TestEmptyNameIsTheDefaultHandler(t *testing.T) {
	resetHandlers()
	defer resetHandlers()

	HandleDefault(func(json.RawMessage) (any, error) { return "ok", nil })
	if _, err := Dispatch("", "null"); err != nil {
		t.Fatalf("empty name should route to the default handler: %v", err)
	}
}

func TestUnknownHandlerNamesTheRegisteredSet(t *testing.T) {
	resetHandlers()
	defer resetHandlers()

	HandleDefault(func(json.RawMessage) (any, error) { return nil, nil })
	Handle("shout", func(json.RawMessage) (any, error) { return nil, nil })

	_, err := Dispatch("nope", "null")
	if err == nil {
		t.Fatal("expected an error for an unknown handler")
	}
	msg := err.Error()
	for _, want := range []string{`unknown handler "nope"`, "default", "shout"} {
		if !strings.Contains(msg, want) {
			t.Fatalf("error %q does not mention %q", msg, want)
		}
	}
}

func TestUnknownHandlerWithNoRegistrations(t *testing.T) {
	resetHandlers()
	defer resetHandlers()

	_, err := Dispatch("nope", "null")
	if err == nil || !strings.Contains(err.Error(), "(none)") {
		t.Fatalf("expected a '(none)' registered set, got %v", err)
	}
}

func TestDuplicateRegistrationPanics(t *testing.T) {
	resetHandlers()
	defer resetHandlers()

	HandleDefault(func(json.RawMessage) (any, error) { return nil, nil })
	defer func() {
		if recover() == nil {
			t.Fatal("registering the same handler name twice must panic")
		}
	}()
	HandleDefault(func(json.RawMessage) (any, error) { return nil, nil })
}

func TestBlankInputBecomesNull(t *testing.T) {
	resetHandlers()
	defer resetHandlers()

	HandleDefault(func(input json.RawMessage) (any, error) { return string(input), nil })
	out, err := Dispatch("default", "")
	if err != nil {
		t.Fatalf("blank input: %v", err)
	}
	if out != `"null"` {
		t.Fatalf("expected the handler to see null, got %s", out)
	}
}

func TestRegisteredHandlersIsSorted(t *testing.T) {
	resetHandlers()
	defer resetHandlers()

	Handle("zeta", func(json.RawMessage) (any, error) { return nil, nil })
	Handle("alpha", func(json.RawMessage) (any, error) { return nil, nil })
	got := RegisteredHandlers()
	if len(got) != 2 || got[0] != "alpha" || got[1] != "zeta" {
		t.Fatalf("expected sorted names, got %v", got)
	}
}
