//! The one error type every SDK call answers with.
//!
//! The Rust and Go SDKs follow the Starlark convention: **every** host error is
//! an `Err`. (The TypeScript SDK is the exception — it reproduces QuickJS's
//! per-method conventions by reusing `api_wrapper.js` verbatim.)

use core::fmt;

/// Result of any RaisinDB SDK call.
pub type Result<T> = core::result::Result<T, Error>;

/// Why a RaisinDB SDK call failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The host answered `Err`, or answered `Ok` with an `{"error": true, ...}`
    /// envelope (the QuickJS convention, which a host method may still use).
    Host(String),
    /// The reply could not be parsed, or an argument could not be serialised.
    Wire(String),
    /// The handler's own input/output JSON did not match its Rust types.
    Payload(String),
    /// The host ABI is older than the one this SDK was generated against.
    Abi(String),
}

impl Error {
    /// A host-side failure with `message`.
    pub fn host(message: impl Into<String>) -> Self {
        Self::Host(message.into())
    }

    /// A serialisation/deserialisation failure with `message`.
    pub fn wire(message: impl Into<String>) -> Self {
        Self::Wire(message.into())
    }

    /// A handler input/output mismatch with `message`.
    pub fn payload(message: impl Into<String>) -> Self {
        Self::Payload(message.into())
    }

    /// The message without the variant prefix.
    pub fn message(&self) -> &str {
        match self {
            Self::Host(m) | Self::Wire(m) | Self::Payload(m) | Self::Abi(m) => m,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Host(m) => write!(f, "{m}"),
            Self::Wire(m) => write!(f, "wire error: {m}"),
            Self::Payload(m) => write!(f, "payload error: {m}"),
            Self::Abi(m) => write!(f, "abi error: {m}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::Wire(e.to_string())
    }
}

impl From<Error> for String {
    fn from(e: Error) -> Self {
        e.to_string()
    }
}
