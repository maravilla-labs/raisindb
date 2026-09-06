//! The host surface: one generic gateway plus logging, context and the ABI version.
//!
//! On a wasm target these go straight to the WIT imports. On a native target —
//! `cargo test` — they go to the thread-local [`crate::testing::MockHost`], so
//! the same handler code is testable without a server or a wasm runtime.

use crate::error::{Error, Result};

/// Severity of a host log line. Mirrors the WIT `log-level` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    /// Diagnostic detail.
    Debug,
    /// Normal progress.
    Info,
    /// Something suspicious but survivable.
    Warn,
    /// A failure.
    Error,
}

/// Everything the host provides a function component.
///
/// Implemented once per target family; a test double implements it too
/// ([`crate::testing::MockHost`]).
pub trait Host {
    /// Call a RaisinDB API method by registry `internal_name`, with `args` a
    /// JSON array of positional arguments.
    fn call(&self, method: &str, args: &str) -> Result<String>;
    /// Emit a structured log line.
    fn log(&self, level: LogLevel, message: &str);
    /// The execution context JSON, byte-identical to `raisin.context.get()`.
    fn context(&self) -> String;
    /// The host ABI semver.
    fn abi_version(&self) -> String;
}

/// Call a RaisinDB API method by registry `internal_name`.
///
/// `args_json` is a JSON array of positional arguments (`null` = absent
/// optional). Prefer the typed wrappers in the generated modules; this is the
/// escape hatch for a method the SDK has not been regenerated for.
pub fn call(method: &str, args_json: &str) -> Result<String> {
    imp::call(method, args_json)
}

/// Emit a structured log line. Reaches `ExecutionResult.logs` and the SSE stream.
pub fn log(level: LogLevel, message: &str) {
    imp::log(level, message)
}

/// The raw execution-context JSON. See [`crate::context`] for the typed view.
pub fn context() -> String {
    imp::context()
}

/// The host's ABI semver, e.g. `"0.1.0"`.
pub fn abi_version() -> String {
    imp::abi_version()
}

/// The ABI version this SDK's bindings were generated against.
pub const SDK_ABI_VERSION: &str = "0.1.0";

/// Fail unless the host's ABI major version matches [`SDK_ABI_VERSION`]'s.
///
/// Call it from a handler that depends on a recent host method; a mismatch is
/// otherwise only visible as "unknown raisin API method".
pub fn require_abi() -> Result<()> {
    let host = abi_version();
    let major = |v: &str| v.split('.').next().unwrap_or_default().to_string();
    if major(&host) == major(SDK_ABI_VERSION) {
        Ok(())
    } else {
        Err(Error::Abi(format!(
            "host ABI {host} is incompatible with SDK ABI {SDK_ABI_VERSION}"
        )))
    }
}

#[cfg(target_family = "wasm")]
mod imp {
    use super::LogLevel;
    use crate::bindings::raisin::function::host as wit;
    use crate::error::{Error, Result};

    pub fn call(method: &str, args: &str) -> Result<String> {
        wit::call(method, args).map_err(Error::Host)
    }

    pub fn log(level: LogLevel, message: &str) {
        wit::log(level.into(), message)
    }

    pub fn context() -> String {
        wit::context()
    }

    pub fn abi_version() -> String {
        wit::abi_version()
    }

    impl From<LogLevel> for wit::LogLevel {
        fn from(level: LogLevel) -> Self {
            match level {
                LogLevel::Debug => Self::Debug,
                LogLevel::Info => Self::Info,
                LogLevel::Warn => Self::Warn,
                LogLevel::Error => Self::Error,
            }
        }
    }
}

#[cfg(not(target_family = "wasm"))]
mod imp {
    use super::LogLevel;
    use crate::error::Result;
    use crate::testing;

    pub fn call(method: &str, args: &str) -> Result<String> {
        testing::host_call(method, args)
    }

    pub fn log(level: LogLevel, message: &str) {
        testing::host_log(level, message)
    }

    pub fn context() -> String {
        testing::host_context()
    }

    pub fn abi_version() -> String {
        testing::host_abi_version()
    }
}
