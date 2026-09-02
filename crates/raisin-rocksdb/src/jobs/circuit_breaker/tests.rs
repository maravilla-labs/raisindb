//! Breaker transitions. The properties here are the ones the incident turned
//! on, so each test names the failure it is holding back.

use super::*;
use std::time::Duration;

const KEY: &str = "embedding:openai:https://marvel.example";

fn registry(cooldown_ms: u64) -> CircuitBreakerRegistry {
    CircuitBreakerRegistry::with_policy(BreakerPolicy {
        failure_threshold: 5,
        cooldown: Duration::from_millis(cooldown_ms),
        probe_timeout: Duration::from_secs(120),
    })
}

fn upstream_5xx() -> Error {
    Error::Upstream {
        upstream: "OpenAI".to_string(),
        status: Some(503),
        message: "Service Unavailable".to_string(),
    }
}

fn client_4xx() -> Error {
    Error::Upstream {
        upstream: "OpenAI".to_string(),
        status: Some(401),
        message: "Invalid API key".to_string(),
    }
}

/// (a) N consecutive 5xx open it — and not N-1.
///
/// The threshold is what turns 476 identical 503s into five: everything after
/// the fifth parks instead of paying for its own discovery.
#[test]
fn consecutive_5xx_open_the_breaker() {
    let reg = registry(60_000);

    for _ in 0..4 {
        assert!(reg.admit(KEY).is_ok());
        reg.record_failure(KEY, &upstream_5xx());
    }
    assert!(
        reg.admit(KEY).is_ok(),
        "four faults must not open it — the threshold is five"
    );
    assert_eq!(reg.status(KEY).unwrap().state, BreakerState::Closed);

    reg.record_failure(KEY, &upstream_5xx());

    let status = reg.status(KEY).unwrap();
    assert_eq!(status.state, BreakerState::Open);
    assert_eq!(status.consecutive_failures, 5);
    assert!(status.last_error.unwrap().contains("Service Unavailable"));
    assert!(status.next_probe_in.is_some());

    let err = reg.admit(KEY).unwrap_err();
    assert!(err.is_upstream_unavailable(), "got {err}");
}

/// A transport failure — connect refused, DNS, TLS, timeout — is an upstream
/// fault too. There is no response to attribute to our request.
#[test]
fn transport_failures_open_the_breaker() {
    let reg = registry(60_000);
    let transport = Error::Upstream {
        upstream: "Ollama".to_string(),
        status: None,
        message: "error trying to connect: tcp connect error".to_string(),
    };
    for _ in 0..5 {
        reg.record_failure(KEY, &transport);
    }
    assert_eq!(reg.status(KEY).unwrap().state, BreakerState::Open);
}

/// (b) A 4xx is OURS and must NOT open it.
///
/// The API key is resolved per tenant while the endpoint is shared, so opening
/// on a 401 would let one tenant's expired credential park every other tenant's
/// jobs — the exact harm the breaker exists to prevent, caused by the breaker.
#[test]
fn client_errors_never_open_the_breaker() {
    let reg = registry(60_000);
    for _ in 0..50 {
        assert!(reg.admit(KEY).is_ok());
        reg.record_failure(KEY, &client_4xx());
    }
    let status = reg.status(KEY).unwrap();
    assert_eq!(status.state, BreakerState::Closed);
    assert_eq!(
        status.consecutive_failures, 0,
        "a 4xx must not even be counted towards opening — otherwise one broken \
         tenant config parks everyone by a slower route"
    );
}

/// A 4xx mixed into a run of 5xx does not reset the count, and does not stop
/// the breaker from opening on the fifth genuine fault.
#[test]
fn a_client_error_does_not_reset_the_upstream_count() {
    let reg = registry(60_000);
    for _ in 0..4 {
        reg.record_failure(KEY, &upstream_5xx());
    }
    reg.record_failure(KEY, &client_4xx());
    reg.record_failure(KEY, &upstream_5xx());
    assert_eq!(reg.status(KEY).unwrap().state, BreakerState::Open);
}

/// (d) Half-open admits exactly ONE probe.
///
/// If it admitted more, a recovering upstream would be hit by the whole parked
/// backlog the instant the cooldown elapsed — the thundering herd the breaker
/// was opened to prevent.
#[test]
fn half_open_admits_exactly_one_probe() {
    let reg = registry(10);
    for _ in 0..5 {
        reg.record_failure(KEY, &upstream_5xx());
    }
    assert!(reg.admit(KEY).is_err());

    std::thread::sleep(Duration::from_millis(20));

    assert!(
        reg.admit(KEY).is_ok(),
        "the first caller after the cooldown probes"
    );
    assert_eq!(reg.status(KEY).unwrap().state, BreakerState::HalfOpen);
    for _ in 0..10 {
        assert!(
            reg.admit(KEY).is_err(),
            "everyone else keeps parking until the probe reports"
        );
    }
}

/// A failed probe re-opens and restarts the cooldown: one request per cooldown
/// is the entire budget an unhealthy upstream gets.
#[test]
fn a_failed_probe_reopens_the_breaker() {
    let reg = registry(10);
    for _ in 0..5 {
        reg.record_failure(KEY, &upstream_5xx());
    }
    std::thread::sleep(Duration::from_millis(20));
    assert!(reg.admit(KEY).is_ok());

    reg.record_failure(KEY, &upstream_5xx());

    assert_eq!(reg.status(KEY).unwrap().state, BreakerState::Open);
    assert!(reg.admit(KEY).is_err());
}

/// (e) Success closes it, and clears the history with it.
#[test]
fn a_successful_probe_closes_the_breaker() {
    let reg = registry(10);
    for _ in 0..5 {
        reg.record_failure(KEY, &upstream_5xx());
    }
    std::thread::sleep(Duration::from_millis(20));
    assert!(reg.admit(KEY).is_ok());

    reg.record_success(KEY);

    let status = reg.status(KEY).unwrap();
    assert_eq!(status.state, BreakerState::Closed);
    assert_eq!(status.consecutive_failures, 0);
    assert!(status.last_error.is_none());
    assert!(status.next_probe_in.is_none());
    for _ in 0..10 {
        assert!(reg.admit(KEY).is_ok());
    }
}

/// A probe holder that never reports must not wedge the breaker forever.
#[test]
fn a_lost_probe_is_replaced_after_its_deadline() {
    let reg = CircuitBreakerRegistry::with_policy(BreakerPolicy {
        failure_threshold: 1,
        cooldown: Duration::from_millis(5),
        probe_timeout: Duration::from_millis(10),
    });
    reg.record_failure(KEY, &upstream_5xx());
    std::thread::sleep(Duration::from_millis(10));
    assert!(reg.admit(KEY).is_ok(), "probe admitted");
    assert!(reg.admit(KEY).is_err(), "second caller parks");

    std::thread::sleep(Duration::from_millis(15));
    assert!(
        reg.admit(KEY).is_ok(),
        "the outstanding probe is past its deadline and presumed lost"
    );
}

/// Breakers are per upstream: one going down must not park a healthy one.
#[test]
fn breakers_are_independent_per_upstream() {
    let reg = registry(60_000);
    let other = "embedding:ollama:http://localhost:11434";
    for _ in 0..5 {
        reg.record_failure(KEY, &upstream_5xx());
    }
    assert!(reg.admit(KEY).is_err());
    assert!(reg.admit(other).is_ok());
    assert_eq!(reg.statuses().len(), 2);
}

/// `check` is the read-only question a job asks between units of work. It must
/// never claim the probe — a job asking it twice would otherwise lock itself
/// out halfway through its own attempt.
#[test]
fn check_reports_open_without_claiming_the_probe() {
    let reg = registry(60_000);
    assert!(reg.check(KEY).is_ok(), "an unknown upstream is not open");
    for _ in 0..5 {
        reg.record_failure(KEY, &upstream_5xx());
    }
    assert!(reg.check(KEY).is_err());
    assert_eq!(reg.status(KEY).unwrap().state, BreakerState::Open);
}

/// Two threads racing the first use of an upstream must share ONE cell, or the
/// first N failures split across two counters and the breaker opens late —
/// exactly when the burst is largest.
#[test]
fn concurrent_first_use_shares_one_cell() {
    let reg = Arc::new(registry(60_000));
    let mut handles = Vec::new();
    for _ in 0..8 {
        let reg = Arc::clone(&reg);
        handles.push(std::thread::spawn(move || {
            for _ in 0..10 {
                let _ = reg.admit(KEY);
                reg.record_failure(KEY, &upstream_5xx());
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    let status = reg.status(KEY).unwrap();
    assert_eq!(status.state, BreakerState::Open);
    assert_eq!(status.consecutive_failures, 80);
}
