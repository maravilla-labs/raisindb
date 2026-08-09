//! The `submit` lifecycle: its statuses, and its own error classifier.
//!
//! # Why this classifier exists at all
//!
//! Every other path in this engine asks [`AdapterError::is_retryable`], whose
//! default is "unrecognized means transient, so retry". That default is correct
//! for a read and catastrophic for a send: a retried send is a SECOND EMAIL. So
//! `submit` inverts it — only an answer that PROVES no side effect occurred
//! requeues, and everything else, a timeout emphatically included, parks.
//!
//! It is a separate function rather than a flag on `is_retryable` on purpose.
//! The two questions are genuinely different ("could this succeed if I tried
//! again?" versus "did this already happen?"), and one function answering both
//! is how the safe answer to one becomes the unsafe answer to the other.

use super::super::AdapterError;

/// The lifecycle a `submit` command node moves through.
///
/// Authored: `draft`, then `queued`. Engine-owned from there.
pub(super) mod status {
    /// Composed but NOT authorized to send. The drain never touches one.
    ///
    /// The reason a separate status exists rather than "created means send" is
    /// that an agent, a half-built compose UI or a flow step that creates a node
    /// to fill in later must not be able to send mail by existing. It is also
    /// the nodetype's default, so omitting `status` cannot send either.
    pub const DRAFT: &str = "draft";
    /// Authorized. The drain claims exactly these.
    pub const QUEUED: &str = "queued";
    /// CLAIMED, durably, before any network call. Seeing one at the start of a
    /// drain means a previous run died holding it — see
    /// [`super::STALE_SENDING_REASON`].
    pub const SENDING: &str = "sending";
    /// Terminal, successful.
    pub const SENT: &str = "sent";
    /// Terminal, and definitively NOT sent: the provider rejected the request
    /// before acting on it. A human may edit and requeue.
    pub const FAILED: &str = "failed";
    /// Terminal, and genuinely AMBIGUOUS: it may or may not have been sent.
    /// Never retried by anything but a person.
    pub const UNKNOWN: &str = "unknown";
}

/// What one `submit` attempt's outcome means for the command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Disposition {
    /// Back to `queued`. The ONLY disposition that resends, and it is reachable
    /// from exactly one error.
    Requeue,
    /// Terminal `failed`: a definitive pre-effect rejection.
    Failed,
    /// Terminal `unknown`: it may have happened. Never auto-retried.
    Unknown,
}

/// Classify a failed `submit` for the at-most-once protocol.
///
/// The match is EXHAUSTIVE, with no `_` arm, and that is the point: a new
/// [`AdapterError`] variant must be a compile error here. A catch-all would let
/// the next variant silently inherit whichever neighbour it was written next to,
/// and the cost of inheriting `Requeue` by accident is duplicate mail.
pub(super) fn disposition(error: &AdapterError) -> Disposition {
    match error {
        // The provider refused to even look at the request. This is the one
        // answer that proves nothing was sent, which is why it is the one answer
        // that may be sent again.
        AdapterError::RateLimited { .. } => Disposition::Requeue,

        // Definitive pre-effect rejections. The request was understood and
        // declined: a token the provider will not accept, a mount pointing at a
        // resource that cannot send, a command addressing an object that has
        // moved on. None of them can have produced an effect, and none of them
        // can succeed on a retry of the identical request either — so they are
        // terminal, and a person decides what to do.
        AdapterError::AuthExpired | AdapterError::Config(_) | AdapterError::Conflict(_) => {
            Disposition::Failed
        }

        // EVERYTHING ELSE. A 500, a socket hang-up, a timeout, an adapter that
        // died half way, a cursor error that has no meaning here. Each of them
        // is consistent with the provider having accepted the message and the
        // acknowledgement being lost, and none of them can be distinguished from
        // the others through a thrown string. `Transient` is the catch-all
        // `AdapterError::classify` assigns to any unrecognized message, so this
        // arm is where an unfamiliar provider error lands — deliberately, and
        // parked.
        AdapterError::Transient(_) | AdapterError::CursorInvalid(_) => Disposition::Unknown,
    }
}

/// Recorded on a command found stuck in `sending` at the start of a drain.
///
/// It moves to `unknown` and NOT back to `queued`. That is the whole reason the
/// durable claim in step 1 exists: without it, a crash anywhere between "user
/// clicked send" and "provider answered" is an unbounded question. With it, the
/// question is bounded to a single attempt whose id is recorded on the node —
/// answerable by searching the Sent folder, and never answered by guessing.
pub(super) const STALE_SENDING_REASON: &str =
    "this command was claimed for sending and the run that claimed it did not finish. \
     It may or may not have reached the provider, so it has NOT been retried \
     automatically. Check the provider's sent items for the recorded attempt_id, then \
     set status back to 'queued' if it needs sending.";

#[cfg(test)]
mod tests {
    use super::*;

    /// The inversion, stated as a test: only `rate_limited` comes back.
    #[test]
    fn only_rate_limited_requeues() {
        assert_eq!(
            disposition(&AdapterError::RateLimited {
                retry_after_secs: None
            }),
            Disposition::Requeue
        );
        for e in [
            AdapterError::AuthExpired,
            AdapterError::Config("bad".into()),
            AdapterError::Conflict("changed".into()),
        ] {
            assert_eq!(disposition(&e), Disposition::Failed, "{e}");
        }
        for e in [
            AdapterError::Transient("timed out".into()),
            AdapterError::Transient("boom".into()),
            AdapterError::CursorInvalid("n/a".into()),
        ] {
            assert_eq!(disposition(&e), Disposition::Unknown, "{e}");
        }
    }

    /// The inverted default, isolated from the variant list above: the generic
    /// engine judgement says "retry this", and submit must not agree.
    ///
    /// This is the whole stage in one assertion. A timeout is the single most
    /// likely failure of a send and the single most dangerous one to retry.
    #[test]
    fn a_timeout_parks_even_though_the_engine_calls_it_retryable() {
        let timeout = AdapterError::Transient("request timed out after 30s".to_string());
        assert!(
            timeout.is_retryable(),
            "precondition: the generic classifier treats this as retryable"
        );
        assert_eq!(
            disposition(&timeout),
            Disposition::Unknown,
            "a send must NOT inherit the read path's retry default"
        );
    }
}
