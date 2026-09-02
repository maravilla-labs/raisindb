//! The wire projection of a circuit breaker, and nothing else.
//!
//! Its own module because these three strings — `closed`, `open`, `half_open` —
//! are a contract with the admin console, not an implementation detail of the
//! breaker. `BreakerState` may gain a variant or be renamed; the console must
//! keep rendering the same words until someone changes both sides on purpose.
//! Kept apart from `background_jobs_impl` so that contract can be tested
//! without standing up a storage.

use crate::jobs::{BreakerState, BreakerStatus};
use raisin_storage::BreakerHealth;

/// Project one breaker's live state onto the operator-facing shape.
pub(crate) fn breaker_health(status: BreakerStatus) -> BreakerHealth {
    BreakerHealth {
        key: status.key,
        state: state_name(status.state).to_string(),
        consecutive_failures: status.consecutive_failures,
        last_error: status.last_error,
        next_probe_in_secs: status.next_probe_in.map(|remaining| {
            // Round UP. A breaker with 400ms left would otherwise report
            // "next probe in 0s" while still OPEN, which reads as a stall —
            // the opposite of what it means.
            remaining.as_secs() + u64::from(remaining.subsec_nanos() > 0)
        }),
    }
}

fn state_name(state: BreakerState) -> &'static str {
    match state {
        BreakerState::Closed => "closed",
        BreakerState::Open => "open",
        BreakerState::HalfOpen => "half_open",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn status(state: BreakerState, next_probe_in: Option<Duration>) -> BreakerStatus {
        BreakerStatus {
            key: "embeddings:openai".to_string(),
            state,
            consecutive_failures: 5,
            last_error: Some("503".to_string()),
            next_probe_in,
        }
    }

    #[test]
    fn state_names_are_the_console_contract() {
        assert_eq!(state_name(BreakerState::Closed), "closed");
        assert_eq!(state_name(BreakerState::Open), "open");
        assert_eq!(state_name(BreakerState::HalfOpen), "half_open");
    }

    #[test]
    fn a_sub_second_cooldown_never_reports_zero() {
        // An open breaker reporting "0s" is indistinguishable from a stuck one.
        let health = breaker_health(status(BreakerState::Open, Some(Duration::from_millis(400))));
        assert_eq!(health.next_probe_in_secs, Some(1));

        let health = breaker_health(status(
            BreakerState::Open,
            Some(Duration::from_millis(30_001)),
        ));
        assert_eq!(health.next_probe_in_secs, Some(31));
    }

    #[test]
    fn a_closed_breaker_has_no_probe_time() {
        let health = breaker_health(status(BreakerState::Closed, None));
        assert_eq!(health.state, "closed");
        assert_eq!(health.next_probe_in_secs, None);
    }
}
