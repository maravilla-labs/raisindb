// greet-as — a RaisinDB function in AssemblyScript.
//
// The component exports ONE function. The node's `entry_file` suffix picks the
// handler, so routing is an ordinary comparison — add a case and a Function
// node pointing at `main.wasm:<name>` to serve several from one artifact.

import { run, log, cabi_realloc, unknownHandler, nodes } from "../node_modules/@raisindb/function-assemblyscript/assembly/index";

/** The "default" handler. Takes and returns JSON TEXT. */
function greet(input: string): string {
  log.info("greet-as running");

  // Every raisin.* method is available and returns raw JSON.
  const children = nodes.getChildren("content", "/pages", 50);

  return '{"greeting":"hello","children":' + children + '}';
}

function route(name: string, input: string): string {
  if (name == "default") return greet(input);
  return unknownHandler(name, "default");
}

// Both exports are looked up BY NAME by `wasm-tools component new`.
export function handler(np: i32, nl: i32, ip: i32, il: i32): i32 {
  return run(np, nl, ip, il, route);
}
export { cabi_realloc };
