//! A native test double for the host — `cargo test` without a server or a runtime.
//!
//! Native builds route [`crate::host`] here, so handler code, the generated
//! wrappers and [`crate::transaction::Transaction`] all exercise their real
//! paths; only the WIT boundary is replaced. An unexpected call is an `Err`,
//! never a default answer, so a test that drifts from the code fails loudly.

use crate::error::{Error, Result};
use crate::host::LogLevel;
use std::cell::RefCell;

thread_local! {
    static ACTIVE: RefCell<Option<MockHost>> = const { RefCell::new(None) };
}

/// One recorded gateway call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedCall {
    /// Registry `internal_name`, e.g. `"nodes_get"`.
    pub method: String,
    /// The positional-argument JSON array, as sent.
    pub args: String,
}

struct Expectation {
    method: String,
    /// `None` matches any arguments.
    args: Option<String>,
    result: std::result::Result<String, String>,
    used: bool,
}

/// A scripted host.
pub struct MockHost {
    expectations: Vec<Expectation>,
    calls: Vec<RecordedCall>,
    logs: Vec<(LogLevel, String)>,
    context: String,
    abi_version: String,
}

impl Default for MockHost {
    fn default() -> Self {
        Self::new()
    }
}

impl MockHost {
    /// An empty mock: every call is unexpected until one is scripted.
    pub fn new() -> Self {
        Self {
            expectations: Vec::new(),
            calls: Vec::new(),
            logs: Vec::new(),
            context: "{}".to_string(),
            abi_version: crate::host::SDK_ABI_VERSION.to_string(),
        }
    }

    /// Answer `method` called with exactly `args` (the JSON array, verbatim).
    pub fn expect(
        mut self,
        method: &str,
        args: &str,
        result: std::result::Result<String, String>,
    ) -> Self {
        self.expectations.push(Expectation {
            method: method.to_string(),
            args: Some(args.to_string()),
            result,
            used: false,
        });
        self
    }

    /// Answer `method` regardless of its arguments.
    pub fn expect_any(mut self, method: &str, result: std::result::Result<String, String>) -> Self {
        self.expectations.push(Expectation {
            method: method.to_string(),
            args: None,
            result,
            used: false,
        });
        self
    }

    /// Set the JSON [`crate::context::Context::get`] will read.
    pub fn with_context(mut self, context: &str) -> Self {
        self.context = context.to_string();
        self
    }

    /// Set the ABI version the host reports.
    pub fn with_abi_version(mut self, version: &str) -> Self {
        self.abi_version = version.to_string();
        self
    }

    /// Every gateway call made while this mock was installed, in order.
    pub fn calls(&self) -> &[RecordedCall] {
        &self.calls
    }

    /// Every log line emitted while this mock was installed, in order.
    pub fn logs(&self) -> &[(LogLevel, String)] {
        &self.logs
    }

    /// The expectations that were never matched.
    pub fn unmet(&self) -> Vec<&str> {
        self.expectations
            .iter()
            .filter(|e| !e.used)
            .map(|e| e.method.as_str())
            .collect()
    }

    fn call(&mut self, method: &str, args: &str) -> Result<String> {
        self.calls.push(RecordedCall {
            method: method.to_string(),
            args: args.to_string(),
        });
        let hit = self
            .expectations
            .iter_mut()
            .find(|e| !e.used && e.method == method && e.args.as_deref().is_none_or(|a| a == args));
        match hit {
            Some(e) => {
                e.used = true;
                e.result.clone().map_err(Error::Host)
            }
            None => Err(Error::Host(format!(
                "unexpected host call {method}({args}); script it with MockHost::expect"
            ))),
        }
    }
}

/// Run `body` with `mock` installed, and hand the mock back for assertions.
pub fn with_mock<R>(mock: MockHost, body: impl FnOnce() -> R) -> (R, MockHost) {
    install(mock);
    let out = body();
    (out, take().expect("mock was installed"))
}

/// Install `mock` for this thread, replacing any previous one.
pub fn install(mock: MockHost) {
    ACTIVE.with(|slot| *slot.borrow_mut() = Some(mock));
}

/// Remove and return this thread's mock.
pub fn take() -> Option<MockHost> {
    ACTIVE.with(|slot| slot.borrow_mut().take())
}

pub(crate) fn host_call(method: &str, args: &str) -> Result<String> {
    ACTIVE.with(|slot| match slot.borrow_mut().as_mut() {
        Some(mock) => mock.call(method, args),
        None => Err(Error::Host(format!(
            "no MockHost installed; {method} cannot be answered on a native target"
        ))),
    })
}

pub(crate) fn host_log(level: LogLevel, message: &str) {
    ACTIVE.with(|slot| {
        if let Some(mock) = slot.borrow_mut().as_mut() {
            mock.logs.push((level, message.to_string()));
        }
    });
}

pub(crate) fn host_context() -> String {
    ACTIVE.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|m| m.context.clone())
            .unwrap_or_else(|| "{}".to_string())
    })
}

pub(crate) fn host_abi_version() -> String {
    ACTIVE.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|m| m.abi_version.clone())
            .unwrap_or_else(|| crate::host::SDK_ABI_VERSION.to_string())
    })
}
