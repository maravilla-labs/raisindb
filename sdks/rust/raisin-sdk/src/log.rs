//! Logging. Every line reaches `ExecutionResult.logs` and the SSE log stream.
//!
//! `println!` / `eprintln!` also work — the runtime drains stdout and stderr
//! line-wise into the same log — but they carry no level, so prefer these.

pub use crate::host::LogLevel;

/// Log at `level`.
pub fn log(level: LogLevel, message: impl AsRef<str>) {
    crate::host::log(level, message.as_ref());
}

/// Log at debug level.
pub fn debug(message: impl AsRef<str>) {
    log(LogLevel::Debug, message);
}

/// Log at info level.
pub fn info(message: impl AsRef<str>) {
    log(LogLevel::Info, message);
}

/// Log at warn level.
pub fn warn(message: impl AsRef<str>) {
    log(LogLevel::Warn, message);
}

/// Log at error level.
pub fn error(message: impl AsRef<str>) {
    log(LogLevel::Error, message);
}
