// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

package raisin

import (
	"encoding/json"
	"fmt"
	"sort"
	"strings"
	"sync"
)

// DefaultHandler is the handler name a bare `entry_file: main.wasm` selects.
const DefaultHandler = "default"

// HandlerFunc handles one invocation. The input is the raw JSON the caller
// sent; the returned value is JSON-encoded as the function output.
type HandlerFunc func(input json.RawMessage) (any, error)

var (
	handlersMu sync.RWMutex
	handlers   = map[string]HandlerFunc{}
)

// Handle registers fn under name. The name is what a `raisin:Function` node's
// `entry_file` suffix selects (`main.wasm:on-order` -> "on-order"), so one
// artifact can back many Function nodes. Registering the same name twice
// panics: a silent overwrite would make the losing handler unreachable with no
// error anywhere.
func Handle(name string, fn HandlerFunc) {
	if name == "" {
		panic("raisin: handler name must not be empty")
	}
	if fn == nil {
		panic("raisin: handler " + name + " must not be nil")
	}
	handlersMu.Lock()
	defer handlersMu.Unlock()
	if _, exists := handlers[name]; exists {
		panic("raisin: handler " + name + " is already registered")
	}
	handlers[name] = fn
}

// HandleDefault registers fn as the "default" handler, which is what a bare
// `entry_file: main.wasm` (no `:handler` suffix) selects.
func HandleDefault(fn HandlerFunc) {
	Handle(DefaultHandler, fn)
}

// RegisteredHandlers returns the registered handler names, sorted.
func RegisteredHandlers() []string {
	handlersMu.RLock()
	defer handlersMu.RUnlock()
	names := make([]string, 0, len(handlers))
	for name := range handlers {
		names = append(names, name)
	}
	sort.Strings(names)
	return names
}

// Dispatch routes one `handler(name, input)` export call. It is the body of
// the single WIT export; tests call it directly to exercise a named handler.
//
// An unknown name is an error naming everything the guest registered — the
// host never validates handler names, so this message is the only diagnosis a
// typo'd `entry_file` ever gets.
func Dispatch(name string, input string) (string, error) {
	if name == "" {
		name = DefaultHandler
	}
	handlersMu.RLock()
	fn, ok := handlers[name]
	handlersMu.RUnlock()
	if !ok {
		return "", fmt.Errorf("unknown handler %q; registered: %s", name, registeredList())
	}

	payload := json.RawMessage(input)
	if strings.TrimSpace(input) == "" {
		payload = json.RawMessage("null")
	}
	out, err := fn(payload)
	if err != nil {
		return "", err
	}
	encoded, err := json.Marshal(out)
	if err != nil {
		return "", fmt.Errorf("handler %q returned a value that is not JSON: %w", name, err)
	}
	return string(encoded), nil
}

// registeredList renders the registered names for an error message.
func registeredList() string {
	names := RegisteredHandlers()
	if len(names) == 0 {
		return "(none)"
	}
	return strings.Join(names, ", ")
}

// resetHandlers drops every registration. Test-only.
func resetHandlers() {
	handlersMu.Lock()
	handlers = map[string]HandlerFunc{}
	handlersMu.Unlock()
}
