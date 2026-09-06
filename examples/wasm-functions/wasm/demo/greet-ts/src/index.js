// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

/**
 * greet-ts — a RaisinDB function compiled to a WebAssembly component.
 *
 * Note what is NOT here: no import of the SDK, no host plumbing, no
 * registration call. This is an ordinary QuickJS function — the same
 * `globalThis.raisin` surface, the same `console`, the same per-method error
 * conventions — and componentizing it is a build step, not a rewrite.
 *
 * It exports TWO handlers into the one `handler(name, input)` export, so a
 * single main.wasm can back several `raisin:Function` nodes:
 *
 *   entry_file: main.wasm                    -> "default" -> handler
 *   entry_file: main.wasm:shout              -> "shout"   -> shout
 *   entry_file: ../greet-ts/main.wasm:shout  (from a sibling node directory)
 */

/** Greet the caller and count the nodes under /people. */
export async function handler(input) {
  const { name, people } = load(input);
  console.log('greeting', name, `(${people} people)`);
  return { greeting: `Hello, ${name}!`, people, language: 'ts' };
}

/** The same greeting in upper case: one artifact, two handlers. */
export async function shout(input) {
  const { name, people } = load(input);
  console.log('shouting at', name);
  return { greeting: `Hello, ${name}!`.toUpperCase(), people, language: 'ts' };
}

function load(input) {
  const name = input && input.name;
  if (!name) throw new Error('input.name is required');
  // getChildren answers [] on failure — the frozen QuickJS convention, which
  // is the same code here as in the JavaScript runtime.
  const children = raisin.nodes.getChildren('content', '/people');
  return { name, people: children.length };
}
