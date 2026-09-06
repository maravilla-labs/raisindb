//! # raisin-sdk — write a RaisinDB function as a WebAssembly component in Rust
//!
//! ```ignore
//! use raisin_sdk::prelude::*;
//!
//! #[derive(serde::Deserialize)]
//! struct Input { name: String }
//! #[derive(serde::Serialize)]
//! struct Output { greeting: String }
//!
//! #[raisin_sdk::handler]                      // -> handler name "default"
//! fn greet(input: Input) -> Result<Output> {
//!     raisin_sdk::log::info(format!("greeting {}", input.name));
//!     Ok(Output { greeting: format!("Hello, {}!", input.name) })
//! }
//!
//! #[raisin_sdk::handler(name = "shout")]      // -> handler name "shout"
//! fn shout(input: Input) -> Result<Output> {
//!     Ok(Output { greeting: format!("HELLO, {}!", input.name.to_uppercase()) })
//! }
//!
//! raisin_sdk::export!(greet, shout);
//! ```
//!
//! ## One artifact, N handlers
//!
//! The WIT export is fixed — `handler(name, input)` — and the handler *name* is
//! data, chosen by the Function node's `entry_file` suffix: `main.wasm` selects
//! `"default"`, `main.wasm:shout` selects `"shout"`, and a second Function node
//! may point at the same artifact with `../greet/main.wasm:shout`. The host
//! never checks the name against a list; [`dispatch`] answers an unknown one
//! with an `Err` naming what this component registered.
//!
//! ## Registration
//!
//! [`macro@handler`] names a handler; [`export!`] lists them and builds the
//! dispatch table. The list is explicit rather than link-time
//! (`inventory`/`linkme`) because `wasm32-wasip2` has no constructor support
//! this SDK relies on, and a distributed slice that came back empty would be
//! indistinguishable from a component that registered nothing.
//!
//! ## Building
//!
//! ```text
//! rustup target add wasm32-wasip2
//! cargo build --release --target wasm32-wasip2     # native component output
//! ```
//!
//! `cargo component build --release` works too; neither is required by the
//! other. `crate-type = ["cdylib"]`, and the release profile here already sets
//! `opt-level = "s"`, `lto`, `panic = "abort"` and `strip` (~200-400 KB).
//!
//! ## Testing
//!
//! `cargo test` builds natively: [`host`] routes to [`testing::MockHost`], so
//! handlers, the generated wrappers and [`transaction::Transaction`] are all
//! exercised without a server, a wasm runtime, or a network.

#![warn(missing_docs)]
#![allow(clippy::needless_doctest_main)]

#[cfg(target_family = "wasm")]
pub mod bindings;

pub mod context;
mod error;
mod handler;
pub mod host;
pub mod log;
pub mod transaction;
pub mod wire;

#[cfg(not(target_family = "wasm"))]
pub mod testing;

#[doc(hidden)]
pub mod __private;

/// Typed wrappers over every `raisin.*` host method — GENERATED, do not edit.
///
/// Regenerate with `make gen-bindings`; the source of truth is
/// `crates/raisin-functions/src/runtime/bindings/`.
///
/// Its per-category modules carry no module-level docs (the generator emits
/// per-function docs only), so `missing_docs` is relaxed here rather than in
/// the machine-owned file.
#[allow(missing_docs)]
mod generated;

pub use error::{Error, Result};
pub use generated::*;
pub use handler::{dispatch, registered_names, Handler};
pub use host::LogLevel;
pub use raisin_sdk_macros::{export, handler};
pub use transaction::Transaction;

/// The imports most functions want.
pub mod prelude {
    pub use crate::context::Context;
    pub use crate::transaction::Transaction;
    pub use crate::{Error, LogLevel, Result};
}
