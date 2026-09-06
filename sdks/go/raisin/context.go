// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

package raisin

import (
	"encoding/json"
	"fmt"
)

// ExecutionContext is the subset of the execution context every runtime
// guarantees. Use ContextInto for the full, deployment-specific shape.
type ExecutionContext struct {
	Tenant    string `json:"tenant_id"`
	Repo      string `json:"repo_id"`
	Branch    string `json:"branch"`
	User      string `json:"user_id"`
	Function  string `json:"function_path"`
	Trigger   string `json:"trigger"`
	RequestID string `json:"request_id"`
}

// ContextJSON returns the raw execution-context JSON, byte-identical to
// `raisin.context.get()` in JavaScript and Starlark.
func ContextJSON() json.RawMessage {
	return json.RawMessage(currentHost().Context())
}

// Context decodes the execution context into ExecutionContext.
func Context() (ExecutionContext, error) {
	var ctx ExecutionContext
	if err := json.Unmarshal(ContextJSON(), &ctx); err != nil {
		return ctx, fmt.Errorf("raisin: cannot decode execution context: %w", err)
	}
	return ctx, nil
}

// ContextInto decodes the execution context into a caller-supplied type.
func ContextInto[T any]() (T, error) {
	var out T
	if err := json.Unmarshal(ContextJSON(), &out); err != nil {
		return out, fmt.Errorf("raisin: cannot decode execution context: %w", err)
	}
	return out, nil
}
