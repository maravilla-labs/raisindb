// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Redis-backed distributed [`LockManager`].
//!
//! Multiple RaisinDB nodes pointed at one linearizable Redis share a single
//! source of truth. All check-then-act operations run as atomic Lua scripts so
//! there is no client-side race window. Acquire uses `SET ... PX` so Redis owns
//! lease expiry; the fencing token comes from a monotonic `INCR` counter.

use std::time::Duration;

use async_trait::async_trait;
use raisin_error::{Error, Result};
use redis::aio::ConnectionManager;
use redis::Script;

use crate::{now_ms, FencingToken, LockGuard, LockManager, RedisConfig};

/// Acquire: take the key iff free, mint a fencing token. Returns token or nil.
const ACQUIRE_LUA: &str = r#"
if redis.call('GET', KEYS[1]) then return nil end
local token = redis.call('INCR', KEYS[2])
redis.call('SET', KEYS[1], ARGV[1] .. ':' .. token, 'PX', tonumber(ARGV[2]))
return token
"#;

/// Release iff the trailing token matches. Returns 1 if released, else 0.
const RELEASE_LUA: &str = r#"
local cur = redis.call('GET', KEYS[1])
if not cur then return 0 end
if string.match(cur, '([^:]+)$') == ARGV[1] then
  redis.call('DEL', KEYS[1])
  return 1
end
return 0
"#;

/// Renew the lease iff the trailing token matches. Returns 1 if renewed, else 0.
const RENEW_LUA: &str = r#"
local cur = redis.call('GET', KEYS[1])
if not cur then return 0 end
if string.match(cur, '([^:]+)$') == ARGV[1] then
  redis.call('PEXPIRE', KEYS[1], tonumber(ARGV[2]))
  return 1
end
return 0
"#;

/// Claim n units, seeding to capacity on first touch. Returns remaining or -1.
const CLAIM_LUA: &str = r#"
if redis.call('EXISTS', KEYS[1]) == 0 then
  redis.call('SET', KEYS[1], tonumber(ARGV[2]))
end
local cur = tonumber(redis.call('GET', KEYS[1]))
local n = tonumber(ARGV[1])
if cur < n then return -1 end
return redis.call('DECRBY', KEYS[1], n)
"#;

/// Return n units to the pool. Returns the new remaining count.
const RELEASE_CLAIM_LUA: &str = r#"
if redis.call('EXISTS', KEYS[1]) == 0 then
  redis.call('SET', KEYS[1], 0)
end
return redis.call('INCRBY', KEYS[1], tonumber(ARGV[1]))
"#;

/// Distributed lock manager over Redis.
#[derive(Clone)]
pub struct RedisLockManager {
    conn: ConnectionManager,
    namespace: String,
}

fn map_err(e: redis::RedisError) -> Error {
    Error::Backend(format!("redis lock backend: {e}"))
}

impl RedisLockManager {
    /// Connect using the supplied config.
    pub async fn connect(config: &RedisConfig) -> Result<Self> {
        let client = redis::Client::open(config.url.as_str()).map_err(map_err)?;
        let conn = ConnectionManager::new(client).await.map_err(map_err)?;
        Ok(Self {
            conn,
            namespace: config.namespace.clone(),
        })
    }

    fn lock_key(&self, key: &str) -> String {
        format!("{}:lock:{}", self.namespace, key)
    }

    fn fence_key(&self) -> String {
        format!("{}:fence", self.namespace)
    }

    fn pool_key(&self, pool: &str) -> String {
        format!("{}:pool:{}", self.namespace, pool)
    }
}

#[async_trait]
impl LockManager for RedisLockManager {
    async fn try_acquire(
        &self,
        key: &str,
        owner: &str,
        ttl: Duration,
    ) -> Result<Option<LockGuard>> {
        let ttl_ms = ttl.as_millis() as u64;
        let mut conn = self.conn.clone();
        let token: Option<i64> = Script::new(ACQUIRE_LUA)
            .key(self.lock_key(key))
            .key(self.fence_key())
            .arg(owner)
            .arg(ttl_ms)
            .invoke_async(&mut conn)
            .await
            .map_err(map_err)?;
        Ok(token.map(|t| LockGuard {
            key: key.to_string(),
            token: t as FencingToken,
            expires_at_ms: now_ms().saturating_add(ttl_ms),
        }))
    }

    async fn release(&self, key: &str, token: FencingToken) -> Result<bool> {
        let mut conn = self.conn.clone();
        let released: i64 = Script::new(RELEASE_LUA)
            .key(self.lock_key(key))
            .arg(token.to_string())
            .invoke_async(&mut conn)
            .await
            .map_err(map_err)?;
        Ok(released == 1)
    }

    async fn renew(&self, key: &str, token: FencingToken, ttl: Duration) -> Result<bool> {
        let mut conn = self.conn.clone();
        let renewed: i64 = Script::new(RENEW_LUA)
            .key(self.lock_key(key))
            .arg(token.to_string())
            .arg(ttl.as_millis() as u64)
            .invoke_async(&mut conn)
            .await
            .map_err(map_err)?;
        Ok(renewed == 1)
    }

    async fn claim(&self, pool: &str, n: u64, capacity: u64) -> Result<Option<u64>> {
        let mut conn = self.conn.clone();
        let remaining: i64 = Script::new(CLAIM_LUA)
            .key(self.pool_key(pool))
            .arg(n)
            .arg(capacity)
            .invoke_async(&mut conn)
            .await
            .map_err(map_err)?;
        if remaining < 0 {
            Ok(None)
        } else {
            Ok(Some(remaining as u64))
        }
    }

    async fn release_claim(&self, pool: &str, n: u64) -> Result<u64> {
        let mut conn = self.conn.clone();
        let remaining: i64 = Script::new(RELEASE_CLAIM_LUA)
            .key(self.pool_key(pool))
            .arg(n)
            .invoke_async(&mut conn)
            .await
            .map_err(map_err)?;
        Ok(remaining.max(0) as u64)
    }
}
