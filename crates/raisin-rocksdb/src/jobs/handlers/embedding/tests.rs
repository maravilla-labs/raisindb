//! The one property the circuit breaker exists for: with the breaker OPEN, the
//! embedding job does no work at all.
//!
//! The breaker's own transitions are covered in `jobs::circuit_breaker::tests`.
//! What those cannot show is that the handler CONSULTS it early enough — a gate
//! placed just before the HTTP call would pass every one of them while still
//! chunking a 40-page document 2,512 times, which is exactly the incident.
//!
//! # How "did no work" is asserted without a fake provider
//!
//! By ordering. The job's steps are: resolve the provider → **gate** → fetch
//! the node → build the embed tasks → resolve chunking → chunk → call the
//! upstream. The node is deliberately never created, so:
//!
//! * breaker CLOSED → the job gets past the gate and dies at the node fetch
//!   with `NotFound`. That is proof the gate is open and that everything after
//!   it would have run.
//! * breaker OPEN → `UpstreamUnavailable`, and the node fetch never happens.
//!
//! The two together pin the gate to a position: strictly before the node is
//! read, and therefore strictly before anything is chunked. A gate moved down
//! to the provider call turns the second case into `NotFound` and fails here.

use std::collections::HashMap;
use std::sync::Arc;

use super::upstream::upstream_key;
use crate::jobs::circuit_breaker::{BreakerPolicy, BreakerState, CircuitBreakerRegistry};
use crate::{EmbeddingJobHandler, RocksDBStorage};
use raisin_embeddings::config::{EmbeddingProvider, TenantEmbeddingConfig};
use raisin_embeddings::resolve::ResolvedEmbeddingProvider;
use raisin_embeddings::TenantEmbeddingConfigStore;
use raisin_error::Error;
use raisin_hnsw::HnswIndexingEngine;
use raisin_storage::jobs::{JobContext, JobId, JobInfo, JobStatus, JobType};
use raisin_storage::{BranchRepository, Storage};
use tempfile::TempDir;

const TENANT: &str = "breaker-tenant";
const REPO: &str = "repo";
const BRANCH: &str = "main";
const WS: &str = "default";
const MASTER_KEY: [u8; 32] = [7u8; 32];
const BASE_URL: &str = "http://127.0.0.1:11434";

struct Env {
    _dir: TempDir,
    storage: Arc<RocksDBStorage>,
    hnsw: Arc<HnswIndexingEngine>,
    revision: raisin_hlc::HLC,
}

impl Env {
    async fn new() -> Env {
        let dir = TempDir::new().expect("temp dir");
        let storage = Arc::new(RocksDBStorage::new(dir.path()).expect("storage"));
        let hnsw = Arc::new(
            HnswIndexingEngine::new(dir.path().join("hnsw"), 8 * 1024 * 1024, 768)
                .expect("hnsw engine"),
        );

        storage
            .branches()
            .create_branch(TENANT, REPO, BRANCH, "test", None, None, false, false)
            .await
            .expect("branch");

        // Ollama, because it is the one provider that needs no API key — this
        // test must never depend on a credential, and it never dials anything.
        let mut config = TenantEmbeddingConfig::new(TENANT.to_string());
        config.enabled = true;
        config.provider = EmbeddingProvider::Ollama;
        config.model = "nomic-embed-text".to_string();
        config.dimensions = 768;
        config.base_url = Some(BASE_URL.to_string());
        storage
            .tenant_embedding_config_repository()
            .set_config(&config)
            .expect("store embedding config");

        let revision = storage
            .branches()
            .get_branch(TENANT, REPO, BRANCH)
            .await
            .expect("get branch")
            .expect("branch exists")
            .head;

        Env {
            _dir: dir,
            storage,
            hnsw,
            revision,
        }
    }

    /// The key the handler will derive for this tenant's configuration.
    ///
    /// Built through the same `upstream_key` the handler uses rather than
    /// spelled out as a literal: a test that hard-codes the key would keep
    /// passing after the derivation changed, while opening a breaker the
    /// handler no longer consults.
    fn upstream(&self) -> String {
        upstream_key(&ResolvedEmbeddingProvider {
            provider: EmbeddingProvider::Ollama,
            api_key: String::new(),
            model: "nomic-embed-text".to_string(),
            base_url: Some(BASE_URL.to_string()),
            dimensions: 768,
        })
    }

    fn handler(&self, breakers: &Arc<CircuitBreakerRegistry>) -> EmbeddingJobHandler {
        EmbeddingJobHandler::new(
            Arc::clone(&self.storage),
            Arc::clone(&self.hnsw),
            MASTER_KEY,
        )
        .with_breakers(Arc::clone(breakers))
    }

    fn context(&self) -> JobContext {
        JobContext {
            tenant_id: TENANT.to_string(),
            repo_id: REPO.to_string(),
            branch: BRANCH.to_string(),
            workspace_id: WS.to_string(),
            revision: self.revision,
            metadata: HashMap::new(),
        }
    }

    fn job(&self) -> JobInfo {
        JobInfo {
            id: JobId::new(),
            job_type: JobType::EmbeddingGenerate {
                node_id: "node-that-does-not-exist".to_string(),
            },
            status: JobStatus::Running,
            tenant: TENANT.to_string(),
            started_at: chrono::Utc::now(),
            completed_at: None,
            progress: None,
            error: None,
            result: None,
            retry_count: 0,
            max_retries: 3,
            last_heartbeat: None,
            timeout_seconds: 300,
            next_retry_at: None,
            executing_since: None,
        }
    }
}

/// A registry whose breakers never leave `Open` on their own, so the assertion
/// cannot race a cooldown.
fn registry() -> Arc<CircuitBreakerRegistry> {
    Arc::new(CircuitBreakerRegistry::with_policy(BreakerPolicy {
        failure_threshold: 5,
        cooldown: std::time::Duration::from_secs(3600),
        probe_timeout: std::time::Duration::from_secs(3600),
    }))
}

fn upstream_5xx() -> Error {
    Error::Upstream {
        upstream: "Ollama".to_string(),
        status: Some(503),
        message: "Service Unavailable".to_string(),
    }
}

/// The control: with the breaker closed the job runs on past the gate.
///
/// Without this half, the open-breaker test below proves nothing — an
/// `UpstreamUnavailable` from a handler that was never going to reach the node
/// anyway would look identical.
#[tokio::test]
async fn a_closed_breaker_lets_the_job_reach_the_work() {
    let env = Env::new().await;
    let breakers = registry();

    let err = env
        .handler(&breakers)
        .handle_generate(&env.job(), &env.context())
        .await
        .expect_err("the node does not exist");

    assert!(
        matches!(err, Error::NotFound(_)),
        "expected the job to get as far as fetching the node, got: {err}"
    );
}

/// (c) An open breaker stops the handler BEFORE it does any of the work.
#[tokio::test]
async fn an_open_breaker_parks_the_job_before_any_work() {
    let env = Env::new().await;
    let breakers = registry();

    for _ in 0..5 {
        breakers.record_failure(&env.upstream(), &upstream_5xx());
    }
    assert_eq!(
        breakers.status(&env.upstream()).unwrap().state,
        BreakerState::Open
    );

    let err = env
        .handler(&breakers)
        .handle_generate(&env.job(), &env.context())
        .await
        .expect_err("an open breaker must refuse the work");

    // NOT `NotFound`: the node was never read, so nothing downstream of it —
    // the embed tasks, the chunking, the provider call — happened either.
    assert!(
        err.is_upstream_unavailable(),
        "expected the job to park before touching the node, got: {err}"
    );
    // And the worker must be able to act on it without parsing the message.
    assert!(err.upstream_retry_after().is_some());
}

/// A breaker opened for ANOTHER upstream must not park this job. Otherwise one
/// dead endpoint stops every tenant on every other endpoint — a breaker that
/// causes the outage it exists to contain.
#[tokio::test]
async fn a_breaker_for_a_different_upstream_does_not_park_the_job() {
    let env = Env::new().await;
    let breakers = registry();

    for _ in 0..5 {
        breakers.record_failure("embedding:openai:vendor-default", &upstream_5xx());
    }

    let err = env
        .handler(&breakers)
        .handle_generate(&env.job(), &env.context())
        .await
        .expect_err("the node does not exist");

    assert!(
        matches!(err, Error::NotFound(_)),
        "expected an unrelated breaker to be ignored, got: {err}"
    );
}
