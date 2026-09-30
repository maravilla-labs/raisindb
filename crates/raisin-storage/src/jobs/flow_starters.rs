// SPDX-License-Identifier: BSL-1.1

//! Who started a flow instance whose node is not written yet.
//!
//! Starting a flow only QUEUES a job; the `raisin:FlowInstance` node appears
//! when that job runs. A client that starts a flow and immediately subscribes
//! to its events therefore asks about an instance that does not exist yet, and
//! an ownership check that reads the node would refuse the very caller who
//! started it. The start path records its caller here, and the check consults
//! this record while the node is still missing.
//!
//! Entries are short-lived: once the node exists the check reads it instead,
//! so a record only has to outlive the queue delay. It is process-local, like
//! the flow event broadcaster whose events it guards.

use dashmap::DashMap;
use once_cell::sync::Lazy;
use std::time::{Duration, Instant};

/// How long a start record stays usable. Far longer than any queue delay.
const STARTER_TTL: Duration = Duration::from_secs(60 * 60);

struct Starter {
    tenant_id: String,
    repo: String,
    user_id: String,
    at: Instant,
}

static STARTERS: Lazy<DashMap<String, Starter>> = Lazy::new(DashMap::new);

/// Record that `user_id` started `instance_id` in `tenant_id`/`repo`.
pub fn record_flow_instance_starter(tenant_id: &str, repo: &str, instance_id: &str, user_id: &str) {
    STARTERS.retain(|_, s| s.at.elapsed() < STARTER_TTL);
    STARTERS.insert(
        instance_id.to_string(),
        Starter {
            tenant_id: tenant_id.to_string(),
            repo: repo.to_string(),
            user_id: user_id.to_string(),
            at: Instant::now(),
        },
    );
}

/// The user who started `instance_id` in `tenant_id`/`repo`, if recorded and
/// not expired. An instance of another tenant or repository answers `None`.
pub fn flow_instance_starter(tenant_id: &str, repo: &str, instance_id: &str) -> Option<String> {
    STARTERS
        .get(instance_id)
        .filter(|s| s.tenant_id == tenant_id && s.repo == repo && s.at.elapsed() < STARTER_TTL)
        .map(|s| s.user_id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starter_is_scoped_to_its_tenant_and_repo() {
        record_flow_instance_starter("t1", "r1", "inst-scoped", "alice");
        assert_eq!(
            flow_instance_starter("t1", "r1", "inst-scoped").as_deref(),
            Some("alice")
        );
        assert_eq!(flow_instance_starter("t2", "r1", "inst-scoped"), None);
        assert_eq!(flow_instance_starter("t1", "r2", "inst-scoped"), None);
        assert_eq!(flow_instance_starter("t1", "r1", "inst-unknown"), None);
    }
}
