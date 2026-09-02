// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Common error types for RaisinDB

use thiserror::Error;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Not found: {0}")]
    NotFound(String),
    #[error("Already exists: {0}")]
    AlreadyExists(String),
    #[error("Validation failed: {0}")]
    Validation(String),
    #[error("Conflict: {0}")]
    Conflict(String),
    #[error("Backend error: {0}")]
    Backend(String),
    #[error("Unauthorized: {0}")]
    Unauthorized(String),
    #[error("Forbidden: {0}")]
    Forbidden(String),
    #[error("Permission denied: {0}")]
    PermissionDenied(String),
    #[error("Lock error: {0}")]
    Lock(String),
    #[error("Encoding error: {0}")]
    Encoding(String),
    #[error("Invalid state: {0}")]
    InvalidState(String),
    #[error("Internal error: {0}")]
    Internal(String),
    /// A spatial index scan hit its per-cell entry budget, so it cannot answer
    /// from the index without answering SHORT.
    ///
    /// Typed rather than a `Backend(String)` because it is not a failure but a
    /// **re-plan signal**: the SQL executor recognises it and degrades to the
    /// spatial fallback (a correct-but-slow row scan) instead of failing the
    /// query. A string match on the message would have made that routing depend
    /// on wording nobody would think to preserve.
    #[error(
        "Spatial index budget exceeded: cell '{cell}' for property '{property}' in workspace \
         '{workspace}' holds more than {limit} entries; refusing to answer from a partial scan"
    )]
    SpatialBudgetExceeded {
        workspace: String,
        property: String,
        cell: String,
        limit: usize,
    },
    /// A call to an EXTERNAL provider failed, carrying WHOSE fault it was.
    ///
    /// Typed rather than a `Backend(String)` for the same reason
    /// [`Error::SpatialBudgetExceeded`] is: it is a **routing signal**, not
    /// merely a message. The embedding circuit breaker decides from it whether
    /// to park every tenant's jobs, and a decision that large must not hang on
    /// a provider happening to spell "API error 503" the way the classifier
    /// greps for. `status` is `None` for a transport failure — connect refused,
    /// TLS, timeout, DNS — which is an upstream fault with no HTTP response to
    /// read a code from.
    #[error("Upstream {upstream} failed ({}): {message}",
            status.map(|s| s.to_string()).unwrap_or_else(|| "transport".to_string()))]
    Upstream {
        /// Human-readable upstream label, e.g. `"OpenAI"`. Not the breaker key.
        upstream: String,
        /// HTTP status if one was received; `None` for a transport failure.
        status: Option<u16>,
        message: String,
    },
    /// The circuit breaker for an upstream is OPEN, so the work was not
    /// attempted at all.
    ///
    /// Not a failure of the job: nothing was tried, so nothing failed. The job
    /// worker recognises it and PARKS the job — reschedules it at
    /// `retry_after_secs` **without** incrementing the retry count — because
    /// burning a retry on an attempt that never happened is how a fleet-wide
    /// outage turns into a fleet-wide permanent failure.
    #[error(
        "Upstream '{upstream}' is unavailable (circuit breaker open); \
             not attempted, retry in {retry_after_secs}s"
    )]
    UpstreamUnavailable {
        /// The breaker key — the upstream's identity, not a tenant's.
        upstream: String,
        retry_after_secs: u64,
    },
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl Error {
    /// Create a storage backend error
    ///
    /// Helper for storage implementations to create Backend errors
    pub fn storage(msg: impl Into<String>) -> Self {
        Self::Backend(msg.into())
    }

    /// Create a lock error
    ///
    /// Helper for converting mutex/lock errors
    pub fn lock(msg: impl Into<String>) -> Self {
        Self::Lock(msg.into())
    }

    /// Create an encoding error
    ///
    /// Helper for string encoding/decoding errors
    pub fn encoding(msg: impl Into<String>) -> Self {
        Self::Encoding(msg.into())
    }

    /// Create an invalid state error
    ///
    /// Helper for unexpected state conditions
    pub fn invalid_state(msg: impl Into<String>) -> Self {
        Self::InvalidState(msg.into())
    }

    /// Create an internal error
    ///
    /// Helper for internal invariant violations
    pub fn internal(msg: impl Into<String>) -> Self {
        Self::Internal(msg.into())
    }

    /// Whether this is the spatial per-cell budget signal.
    ///
    /// The query planner/executor uses it to re-plan onto the spatial fallback.
    /// Every other caller should treat it as an ordinary error.
    pub fn is_spatial_budget_exceeded(&self) -> bool {
        matches!(self, Self::SpatialBudgetExceeded { .. })
    }

    /// Is this an upstream's fault rather than ours?
    ///
    /// This is the predicate a circuit breaker counts, and the asymmetry is the
    /// whole point of asking it:
    ///
    /// * **5xx and transport failures are the upstream's.** Every tenant
    ///   pointed at that endpoint is affected, so learning it once and parking
    ///   the rest is correct.
    /// * **4xx is OURS.** A malformed request, a wrong model name, an expired
    ///   API key — those are per-tenant configuration, and one tenant's bad key
    ///   must never park every other tenant's jobs. That is not a hypothetical:
    ///   the API key is resolved per tenant while the endpoint is shared.
    ///
    /// 429 is deliberately NOT counted, because the rule above is "no 4xx opens
    /// the breaker" and a rule with one exception is a rule nobody remembers.
    /// It is the one 4xx worth revisiting if rate-limit storms ever show up in
    /// the way 503s did.
    pub fn is_upstream_fault(&self) -> bool {
        match self {
            Self::Upstream { status: None, .. } => true,
            Self::Upstream {
                status: Some(code), ..
            } => *code >= 500,
            _ => false,
        }
    }

    /// Was this work skipped because an upstream's breaker is open?
    ///
    /// The job worker asks this to PARK rather than retry. See
    /// [`Error::UpstreamUnavailable`].
    pub fn is_upstream_unavailable(&self) -> bool {
        matches!(self, Self::UpstreamUnavailable { .. })
    }

    /// How long the caller should wait before re-attempting parked work.
    pub fn upstream_retry_after(&self) -> Option<std::time::Duration> {
        match self {
            Self::UpstreamUnavailable {
                retry_after_secs, ..
            } => Some(std::time::Duration::from_secs(*retry_after_secs)),
            _ => None,
        }
    }
}
