// SPDX-License-Identifier: BSL-1.1

//! The visitor-chat limits that span hours or a day, so they must outlive a
//! restart: messages and new sessions per client IP, and the agent's daily
//! token budget.
//!
//! * Per-IP windows are kept in a RocksDB next to the storage
//!   (`visitor-chat-limits`), like the magic-link limiter. Without the
//!   RocksDB backend, or when it cannot be opened, they fall back to this
//!   process's memory: still a cap, just not one that survives a restart.
//! * The token budget is counted where the pipeline records spend: the
//!   `raisin:AICostRecord` nodes under the agent's visitor conversations
//!   (`/agents/<name>/inbox/chats/vchat-*`), summed for the UTC day. That
//!   ledger is persisted and shared by every server; the sum is cached per
//!   process and recounted every [`DAY_TOTAL_MAX_AGE`] or
//!   [`DAY_TOTAL_MAX_SENDS`] messages, whichever comes first.
//!
//! [`DAY_TOTAL_MAX_AGE`]: super::limits::DAY_TOTAL_MAX_AGE
//! [`DAY_TOTAL_MAX_SENDS`]: super::limits::DAY_TOTAL_MAX_SENDS

use std::time::{Duration, Instant};

use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::Storage;

use crate::handler::WsState;

#[cfg(feature = "storage-rocksdb")]
static LIMITER: std::sync::OnceLock<
    Option<tokio::sync::Mutex<raisin_ratelimit::RocksRateLimiter>>,
> = std::sync::OnceLock::new();

#[cfg(feature = "storage-rocksdb")]
fn persisted_limiter(
    storage: &raisin_rocksdb::RocksDBStorage,
) -> Option<&'static tokio::sync::Mutex<raisin_ratelimit::RocksRateLimiter>> {
    LIMITER
        .get_or_init(|| {
            let path = storage.config().path.join("visitor-chat-limits");
            match raisin_ratelimit::RocksRateLimiter::open(&path) {
                Ok(limiter) => Some(tokio::sync::Mutex::new(limiter)),
                Err(e) => {
                    tracing::error!(
                        path = %path.display(),
                        error = %e,
                        "visitor chat: cannot open the persisted limiter; per-IP daily limits are kept in memory"
                    );
                    None
                }
            }
        })
        .as_ref()
}

/// Count one event for `key` if fewer than `limit` happened within `window`.
/// Persisted when the server runs on RocksDB. Returns whether it was allowed.
pub async fn hit<S, B>(state: &WsState<S, B>, key: &str, limit: u32, window: Duration) -> bool
where
    S: Storage + TransactionalStorage + 'static,
    B: raisin_binary::BinaryStorage + 'static,
{
    #[cfg(feature = "storage-rocksdb")]
    if let Some(limiter) = state.rocksdb_storage.as_deref().and_then(persisted_limiter) {
        use raisin_ratelimit::RateLimiter;
        if limit == 0 {
            return false;
        }
        // One writer: the limiter reads, appends and writes back a bucket.
        let limiter = limiter.lock().await;
        return limiter
            .check_rate(key, limit as usize, window)
            .await
            .allowed;
    }
    state.visitor_limits.hit(key, limit, window, Instant::now())
}

/// The UTC day a token budget counts against, `YYYY-MM-DD`.
pub fn today() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

/// The tokens `agent_name`'s visitor conversations used on `day`, summed from
/// the pipeline's cost records. `None` when they cannot be counted (no SQL
/// engine in this build, or the query failed): the caller decides.
pub async fn agent_tokens_on<S, B>(
    state: &WsState<S, B>,
    tenant: &str,
    repo: &str,
    agent_name: &str,
    day: &str,
) -> Option<u64>
where
    S: Storage + TransactionalStorage + 'static,
    B: raisin_binary::BinaryStorage + 'static,
{
    #[cfg(feature = "storage-rocksdb")]
    {
        use futures::StreamExt;
        use raisin_models::auth::AuthContext;
        use raisin_models::nodes::properties::PropertyValue;

        // `agent_name` is a validated `[A-Za-z0-9_-]` segment and `day` is ours.
        let chats = format!("/agents/{agent_name}/inbox/chats");
        let sql = format!(
            "SELECT SUM(CAST(properties->>'total_tokens' AS BIGINT)) AS used FROM ai \
             WHERE node_type = 'raisin:AICostRecord' \
             AND DESCENDANT_OF('{chats}') \
             AND path LIKE '{chats}/vchat-%' \
             AND properties->>'timestamp' >= '{day}'"
        );
        let catalog =
            match raisin_sql_execution::workspace_catalog(state.storage.as_ref(), tenant, repo)
                .await
            {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(error = %e, "visitor chat: cannot count the day's tokens");
                    return None;
                }
            };
        let engine =
            raisin_sql_execution::QueryEngine::new(state.storage.clone(), tenant, repo, "main")
                .with_catalog(catalog)
                .with_auth(AuthContext::system_as("visitor-chat"));
        let mut stream = match engine.execute_batch(&sql).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "visitor chat: cannot count the day's tokens");
                return None;
            }
        };
        let row = match stream.next().await {
            Some(Ok(row)) => row,
            // no row at all: nothing recorded
            None => return Some(0),
            Some(Err(e)) => {
                tracing::warn!(error = %e, "visitor chat: cannot count the day's tokens");
                return None;
            }
        };
        let used = match row.columns.get("used") {
            Some(PropertyValue::Integer(i)) => (*i).max(0) as u64,
            Some(PropertyValue::Float(f)) if f.is_finite() => f.max(0.0) as u64,
            Some(PropertyValue::String(s)) => {
                s.trim().parse::<f64>().map_or(0, |f| f.max(0.0) as u64)
            }
            // SUM over no rows is NULL
            _ => 0,
        };
        Some(used)
    }
    #[cfg(not(feature = "storage-rocksdb"))]
    {
        let _ = (state, tenant, repo, agent_name, day);
        None
    }
}

/// Against a real RocksDB state: the day's sum reads exactly the agent's
/// visitor conversations on that day, and the per-IP window is persisted.
#[cfg(all(test, feature = "storage-rocksdb"))]
mod tests {
    use std::sync::Arc;

    use raisin_models::nodes::properties::PropertyValue;
    use raisin_models::nodes::Node;
    use raisin_storage::{BranchRepository, CreateNodeOptions, NodeRepository, StorageScope};

    use super::*;

    const TENANT: &str = "t_visitor_daily";
    const REPO: &str = "r_visitor_daily";

    type St = raisin_rocksdb::RocksDBStorage;
    type Bn = raisin_binary::FilesystemBinaryStorage;

    async fn state(dir: &std::path::Path) -> Arc<WsState<St, Bn>> {
        let storage = Arc::new(St::new(dir.join("db")).unwrap());
        let _ = storage
            .branches()
            .create_branch(TENANT, REPO, "main", "test", None, None, false, false)
            .await;
        let ws_svc = Arc::new(raisin_core::WorkspaceService::new(storage.clone()));
        ws_svc
            .put(
                TENANT,
                REPO,
                raisin_models::workspace::Workspace::new("ai".to_string()),
            )
            .await
            .unwrap();
        let audit = Arc::new(storage.audit_repository());
        Arc::new(WsState::new(
            storage.clone(),
            Arc::new(raisin_core::RaisinConnection::with_storage(storage.clone())),
            ws_svc,
            Arc::new(Bn::new(dir.join("bin"), Some("/files".into()))),
            crate::handler::WsConfig::default(),
            None,
            Some(storage.clone()),
            None,
            None,
            None,
            None,
            audit,
        ))
    }

    async fn put(
        state: &WsState<St, Bn>,
        path: &str,
        node_type: &str,
        props: &[(&str, PropertyValue)],
    ) {
        let name = path.rsplit('/').next().unwrap().to_string();
        let mut node = Node {
            id: format!("n{}", path.replace('/', "-")),
            name,
            path: path.to_string(),
            node_type: node_type.to_string(),
            ..Default::default()
        };
        for (k, v) in props {
            node.properties.insert(k.to_string(), v.clone());
        }
        state
            .storage
            .nodes()
            .create(
                StorageScope::new(TENANT, REPO, "main", "ai"),
                node,
                CreateNodeOptions {
                    validate_schema: false,
                    validate_parent_allows_child: false,
                    validate_workspace_allows_type: false,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
    }

    async fn cost(state: &WsState<St, Bn>, chat: &str, op: &str, tokens: i64, at: &str) {
        put(
            state,
            &format!("{chat}/{op}/cost-record"),
            "raisin:AICostRecord",
            &[
                ("total_tokens", PropertyValue::Integer(tokens)),
                ("timestamp", PropertyValue::String(at.to_string())),
            ],
        )
        .await;
    }

    #[tokio::test]
    async fn the_day_total_counts_this_agents_visitor_conversations_on_that_day() {
        let dir = tempfile::tempdir().unwrap();
        let state = state(dir.path()).await;
        for p in [
            "/agents",
            "/agents/site",
            "/agents/site/inbox",
            "/agents/site/inbox/chats",
            "/agents/site/inbox/chats/vchat-aaaa",
            "/agents/site/inbox/chats/vchat-aaaa/run-1-op-1",
            "/agents/site/inbox/chats/vchat-aaaa/run-1-op-3",
            "/agents/site/inbox/chats/vchat-aaaa/run-0-op-1",
            "/agents/site/inbox/chats/chat-signed-in",
            "/agents/site/inbox/chats/chat-signed-in/run-2-op-1",
            "/agents/other",
            "/agents/other/inbox",
            "/agents/other/inbox/chats",
            "/agents/other/inbox/chats/vchat-bbbb",
            "/agents/other/inbox/chats/vchat-bbbb/run-3-op-1",
        ] {
            put(&state, p, "raisin:Folder", &[]).await;
        }
        let chat = "/agents/site/inbox/chats/vchat-aaaa";
        cost(
            &state,
            chat,
            "run-1-op-1",
            2000,
            "2026-10-01T08:00:00.000+00:00",
        )
        .await;
        cost(
            &state,
            chat,
            "run-1-op-3",
            3000,
            "2026-10-01T08:00:02.000+00:00",
        )
        .await;
        // yesterday
        cost(
            &state,
            chat,
            "run-0-op-1",
            9000,
            "2026-09-30T23:59:59.000+00:00",
        )
        .await;
        // a signed-in user's chat with the same agent
        cost(
            &state,
            "/agents/site/inbox/chats/chat-signed-in",
            "run-2-op-1",
            7000,
            "2026-10-01T09:00:00.000+00:00",
        )
        .await;
        // a tool's own usage, written by the run executor: its timestamp is
        // stored as a Date, not a string
        put(
            &state,
            &format!("{chat}/run-1-op-1/tool-cost-x"),
            "raisin:AICostRecord",
            &[
                ("total_tokens", PropertyValue::Integer(600)),
                (
                    "timestamp",
                    PropertyValue::Date(
                        chrono::DateTime::parse_from_rfc3339("2026-10-01T08:00:01.000Z")
                            .unwrap()
                            .with_timezone(&chrono::Utc)
                            .into(),
                    ),
                ),
            ],
        )
        .await;
        put(
            &state,
            &format!("{chat}/run-0-op-1/tool-cost-y"),
            "raisin:AICostRecord",
            &[
                ("total_tokens", PropertyValue::Integer(900)),
                (
                    "timestamp",
                    PropertyValue::Date(
                        chrono::DateTime::parse_from_rfc3339("2026-09-30T23:00:00.000Z")
                            .unwrap()
                            .with_timezone(&chrono::Utc)
                            .into(),
                    ),
                ),
            ],
        )
        .await;
        // another agent's visitor
        cost(
            &state,
            "/agents/other/inbox/chats/vchat-bbbb",
            "run-3-op-1",
            4000,
            "2026-10-01T09:00:00.000+00:00",
        )
        .await;

        assert_eq!(
            agent_tokens_on(&state, TENANT, REPO, "site", "2026-10-01").await,
            Some(5600)
        );
        assert_eq!(
            agent_tokens_on(&state, TENANT, REPO, "other", "2026-10-01").await,
            Some(4000)
        );
        assert_eq!(
            agent_tokens_on(&state, TENANT, REPO, "site", "2026-10-02").await,
            Some(0),
            "nothing yet on a new day"
        );
        assert_eq!(
            agent_tokens_on(&state, TENANT, REPO, "nobody", "2026-10-01").await,
            Some(0)
        );
    }

    #[tokio::test]
    async fn a_per_ip_window_allows_its_limit_and_persists_it() {
        let dir = tempfile::tempdir().unwrap();
        let state = state(dir.path()).await;
        let window = Duration::from_secs(3600);
        for _ in 0..3 {
            assert!(hit(&state, "visitor-messages:t:1.2.3.4", 3, window).await);
        }
        assert!(!hit(&state, "visitor-messages:t:1.2.3.4", 3, window).await);
        assert!(hit(&state, "visitor-messages:t:5.6.7.8", 3, window).await);
        assert!(!hit(&state, "visitor-messages:t:9.9.9.9", 0, window).await);
        // persisted: not this process's in-memory window
        assert!(
            state
                .visitor_limits
                .hit("visitor-messages:t:1.2.3.4", 3, window, Instant::now()),
            "the memory fallback was not used"
        );
    }
}
