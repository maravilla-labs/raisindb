// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

// Command greet-go is a RaisinDB function compiled to a WebAssembly component.
//
// It registers TWO handlers into the one exported `handler(name, input)`, so a
// single main.wasm can back several `raisin:Function` nodes:
//
//	entry_file: main.wasm            -> "default"
//	entry_file: main.wasm:shout      -> "shout"
//	entry_file: ../greet-go/main.wasm:shout   (from a sibling node directory)
package main

import (
	"encoding/json"
	"fmt"
	"strings"

	"github.com/maravilla-labs/raisindb/sdks/go/raisin"
)

// input is what the caller sends.
type input struct {
	Name string `json:"name"`
}

// output is what every handler here returns.
type output struct {
	Greeting string `json:"greeting"`
	People   int    `json:"people"`
	Language string `json:"language"`
}

func init() {
	raisin.HandleDefault(greet)
	raisin.Handle("shout", shout)
}

// main is required by Go but never runs: the host calls the component export,
// not a program entry point.
func main() {}

// greet returns a greeting plus the number of nodes under /people.
func greet(raw json.RawMessage) (any, error) {
	in, people, err := load(raw)
	if err != nil {
		return nil, err
	}
	raisin.Info("greeting %s (%d people)", in.Name, people)
	return output{
		Greeting: fmt.Sprintf("Hello, %s!", in.Name),
		People:   people,
		Language: "go",
	}, nil
}

// shout is the same greeting in upper case, showing one artifact serving two
// handlers.
func shout(raw json.RawMessage) (any, error) {
	in, people, err := load(raw)
	if err != nil {
		return nil, err
	}
	raisin.Info("shouting at %s", in.Name)
	return output{
		Greeting: strings.ToUpper(fmt.Sprintf("Hello, %s!", in.Name)),
		People:   people,
		Language: "go",
	}, nil
}

// load decodes the input and counts the nodes under /people through the host
// gateway.
func load(raw json.RawMessage) (input, int, error) {
	var in input
	if err := json.Unmarshal(raw, &in); err != nil {
		return in, 0, fmt.Errorf("invalid input: %w", err)
	}
	if in.Name == "" {
		return in, 0, fmt.Errorf("input.name is required")
	}
	children, err := raisin.Into[[]json.RawMessage](
		raisin.Nodes.GetChildren("content", "/people", nil))
	if err != nil {
		return in, 0, err
	}
	return in, len(children), nil
}
