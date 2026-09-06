/**
 * Rust guest scaffold — a crate that builds to a WebAssembly component with
 * plain `cargo build --target wasm32-wasip2` (no cargo-component needed since
 * Rust 1.82) and runs its handlers natively under `cargo test`.
 */

import type { FileEntry } from '../types.js';
import { snakeIdent, type WasmFnVars } from './shared.js';

function cargoToml(v: WasmFnVars): string {
  const dep =
    v.sdk.kind === 'path'
      ? `raisin-sdk = { path = "${v.sdk.value}" }`
      : `raisin-sdk = "${v.sdk.value}"`;
  return `# Guest crate for the \`${v.name}\` Function node.
#
# Its own workspace: a wasm guest builds for wasm32-wasip2 with a size-tuned
# release profile and must not inherit a host workspace's settings.
[workspace]

[package]
name = "${v.name}"
version = "0.1.0"
edition = "2021"
publish = false

[lib]
# \`cdylib\` is what wasm32-wasip2 links into a component; \`rlib\` additionally
# lets \`tests/\` link the crate natively. It costs nothing in the wasm output.
crate-type = ["cdylib", "rlib"]

[dependencies]
${dep}
serde = { version = "1", features = ["derive"] }
serde_json = "1"

[profile.release]
opt-level = "s"
lto = true
panic = "abort"
strip = true
codegen-units = 1
`;
}

function libRs(v: WasmFnVars): string {
  const ident = snakeIdent(v.handler);
  const attr =
    v.handler === 'default'
      ? '#[raisin_sdk::handler]'
      : `#[raisin_sdk::handler(name = "${v.handler}")]`;
  return `//! \`${v.name}\` — a RaisinDB function compiled to a WebAssembly component.
//!
//! Handlers register BY NAME and share one artifact: the \`entry_file\` suffix on
//! a Function node selects which one answers (\`main.wasm:${v.handler}\`; a bare
//! \`main.wasm\` means \`"default"\`). Add another with
//! \`raisindb create function <other> --lang rust --into ${v.name}\`.
//!
//! Build: \`raisindb function build\` (\`cargo build --release --target wasm32-wasip2\`
//! plus the copy into the Function node directory).

use raisin_sdk::prelude::*;
use serde::{Deserialize, Serialize};

/// What the handler accepts.
#[derive(Deserialize)]
pub struct Input {
    /// Who to greet.
    pub name: String,
}

/// What the handler answers.
#[derive(Serialize, Deserialize)]
pub struct Output {
    /// The greeting.
    pub greeting: String,
    /// Which registered handler answered.
    pub handler: String,
}

/// ${v.description}
${attr}
pub fn ${ident}(input: Input) -> Result<Output> {
    raisin_sdk::log::info(format!("greeting {}", input.name));
    Ok(Output {
        greeting: format!("Hello, {}!", input.name),
        handler: "${v.handler}".to_string(),
    })
}

raisin_sdk::export!(${ident});
`;
}

function testsRs(v: WasmFnVars): string {
  const crate = v.name.replace(/-/g, '_');
  return `//! Native tests — no server, no wasm runtime. \`cargo test\` runs these; the same
//! code compiled for \`wasm32-wasip2\` is what ships.

use ${crate}::{raisin_dispatch, Output};
use raisin_sdk::testing::{with_mock, MockHost};

#[test]
fn the_handler_greets() {
    let (out, mock) = with_mock(MockHost::new(), || {
        raisin_dispatch("${v.handler}", r#"{"name":"Ada"}"#).expect("runs")
    });
    let out: Output = serde_json::from_str(&out).expect("json");
    assert_eq!(out.greeting, "Hello, Ada!");
    assert_eq!(out.handler, "${v.handler}");
    assert_eq!(mock.logs().len(), 1);
}

#[test]
fn an_unknown_handler_names_the_registered_set() {
    let err = raisin_dispatch("nope", "{}").expect_err("unknown");
    assert!(err.contains("${v.handler}"), "{err}");
}
`;
}

function readme(v: WasmFnVars): string {
  return `# ${v.title}

Rust guest for the \`${v.name}\` RaisinDB function, compiled to a WebAssembly
component.

    raisindb function doctor .      # toolchain, entry_file, handler names
    cargo test                      # native tests, no server
    raisindb function build .       # build + copy to ${v.nodeDirRel}

## One artifact, many functions

Handlers are registered by name inside one exported \`handler(name, input)\`.
A second Function node can point at THIS artifact instead of building its own:

    raisindb create function ${v.name}-shout --lang rust --into ${v.name} --handler shout

That writes a node with \`entry_file: ../${v.name}/main.wasm:shout\` and adds the
handler here. The resolved path must stay inside the functions workspace.

## Deploying

\`raisindb function build\` puts \`main.wasm\` in the node directory; from there
\`raisindb sync --watch\` pushes it as an asset and \`raisindb deploy . --install\`
packs it. The source in this directory never ships — \`.rapignore\` holds
\`wasm/\`.
`;
}

/** Every file the Rust scaffold writes, under `projectPath`. */
export function rustFiles(v: WasmFnVars, projectPath: string): FileEntry[] {
  return [
    { path: `${projectPath}/Cargo.toml`, content: cargoToml(v) },
    { path: `${projectPath}/src/lib.rs`, content: libRs(v) },
    { path: `${projectPath}/tests/handlers.rs`, content: testsRs(v) },
    {
      path: `${projectPath}/tests/server.json`,
      content: `${JSON.stringify(
        [{ handler: v.handler, input: { name: 'Ada' }, expect: { greeting: 'Hello, Ada!' } }],
        null,
        2
      )}\n`,
    },
    { path: `${projectPath}/README.md`, content: readme(v) },
  ];
}
