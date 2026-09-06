// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

package raisin

import (
	"encoding/json"
	"errors"
	"fmt"
)

// HostError is a failure reported by the RaisinDB host: an unknown method,
// bad arguments, or an API error. Every generated call returns one on failure,
// per the SDK's "every host error is an error" convention.
type HostError struct {
	// Method is the registry internal name that failed, e.g. "nodes_get".
	Method string
	// Message is the host's human-readable failure text.
	Message string
}

// Error implements the error interface.
func (e *HostError) Error() string {
	return fmt.Sprintf("raisin: %s: %s", e.Method, e.Message)
}

// errorEnvelope is the `{"error": true, ...}` shape some host methods return
// inside an Ok payload (the QuickJS convention). The Go SDK never surfaces it
// as a value; it becomes a HostError like any other failure.
type errorEnvelope struct {
	Error   bool   `json:"error"`
	Message string `json:"message"`
	Code    string `json:"code"`
}

// raw performs the gateway call, marshalling the positional arguments and
// rejecting both an Err result and an `{"error": true}` Ok envelope.
func raw(method string, args []any) (string, error) {
	encoded := "[]"
	if len(args) > 0 {
		b, err := json.Marshal(args)
		if err != nil {
			return "", &HostError{Method: method, Message: "cannot encode arguments: " + err.Error()}
		}
		encoded = string(b)
	}
	payload, err := currentHost().Call(method, encoded)
	if err != nil {
		var he *HostError
		if errors.As(err, &he) {
			return "", he
		}
		return "", &HostError{Method: method, Message: err.Error()}
	}
	if env, ok := decodeErrorEnvelope(payload); ok {
		msg := env.Message
		if msg == "" {
			msg = "host reported an error"
		}
		if env.Code != "" {
			msg = fmt.Sprintf("%s (%s)", msg, env.Code)
		}
		return "", &HostError{Method: method, Message: msg}
	}
	return payload, nil
}

// decodeErrorEnvelope reports whether a payload is an `{"error": true, ...}`
// object, which every SDK treats as a failure rather than a value.
func decodeErrorEnvelope(payload string) (errorEnvelope, bool) {
	if len(payload) == 0 || payload[0] != '{' {
		return errorEnvelope{}, false
	}
	var env errorEnvelope
	if err := json.Unmarshal([]byte(payload), &env); err != nil {
		return errorEnvelope{}, false
	}
	return env, env.Error
}

// callJSON invokes a method whose result is a JSON value (or null).
func callJSON(method string, args []any) (json.RawMessage, error) {
	payload, err := raw(method, args)
	if err != nil {
		return nil, err
	}
	return json.RawMessage(payload), nil
}

// callBool invokes a method whose result is a JSON boolean.
func callBool(method string, args []any) (bool, error) {
	payload, err := raw(method, args)
	if err != nil {
		return false, err
	}
	var v bool
	if err := json.Unmarshal([]byte(payload), &v); err != nil {
		return false, &HostError{Method: method, Message: "expected a boolean, got " + payload}
	}
	return v, nil
}

// callInt64 invokes a method whose result is a JSON number.
func callInt64(method string, args []any) (int64, error) {
	payload, err := raw(method, args)
	if err != nil {
		return 0, err
	}
	var v int64
	if err := json.Unmarshal([]byte(payload), &v); err != nil {
		return 0, &HostError{Method: method, Message: "expected an integer, got " + payload}
	}
	return v, nil
}

// callString invokes a method whose result is a JSON string (or null, which
// becomes the empty string).
func callString(method string, args []any) (string, error) {
	payload, err := raw(method, args)
	if err != nil {
		return "", err
	}
	if payload == "null" {
		return "", nil
	}
	var v string
	if err := json.Unmarshal([]byte(payload), &v); err != nil {
		return "", &HostError{Method: method, Message: "expected a string, got " + payload}
	}
	return v, nil
}

// callVoid invokes a method with no result. The wire value is the literal
// `true`; it is ignored.
func callVoid(method string, args []any) error {
	_, err := raw(method, args)
	return err
}

// Into decodes the JSON result of a generated call into T, so a typed result
// is one call rather than two:
//
//	node, err := raisin.Into[Node](raisin.Nodes.Get("content", "/people/ada"))
func Into[T any](payload json.RawMessage, err error) (T, error) {
	var out T
	if err != nil {
		return out, err
	}
	if len(payload) == 0 || string(payload) == "null" {
		return out, nil
	}
	if err := json.Unmarshal(payload, &out); err != nil {
		return out, fmt.Errorf("raisin: cannot decode result: %w", err)
	}
	return out, nil
}
