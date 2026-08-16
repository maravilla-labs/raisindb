// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! `OneTimeTokenStore` against real storage.
//!
//! Magic-link tokens are ordinary nodes rather than rows in a dedicated column
//! family, which is what lets them replicate for free — but it also means every
//! property this store depends on is a claim about a YAML file and an index,
//! not about a struct the compiler checked. Specifically:
//!
//! * lookup narrows on `token_prefix`, which only works if that property is
//!   actually INDEXED in the shipped `raisin_one_time_token.yaml`;
//! * the node type has to be permitted in `raisin:system`, or every write is
//!   refused at validation time;
//! * `mark_used` has to durably stick, or a consumed link stays redeemable.
//!
//! None of that is observable from a unit test with a mock. So this fixture
//! seeds the workspace and the node type from the REAL embedded definitions —
//! `load_global_workspaces()` and `load_embedded_nodetypes_with_hashes()` — so a
//! change to either YAML that breaks the store fails here rather than in
//! production.
//!
//! The sharpest test in the file is [`a_consumed_token_cannot_be_redeemed_twice`]:
//! everything protecting a magic link from replay lives in `mark_used` landing
//! and `is_valid` consulting it.

use std::collections::HashMap;
use std::sync::Arc;

use raisin_context::RepositoryConfig;
use raisin_core::nodetype_init::load_embedded_nodetypes_with_hashes;
use raisin_core::services::workspace_service::WorkspaceService;
use raisin_core::workspace_init::load_global_workspaces;
use raisin_error::Result;
use raisin_models::auth::{OneTimeToken, TokenPurpose};
use raisin_models::StorageTimestamp;
use raisin_rocksdb::one_time_token::OneTimeTokenStore;
use raisin_rocksdb::{RocksDBConfig, RocksDBStorage};
use raisin_storage::{
    BranchRepository, RegistryRepository, RepositoryManagementRepository, Storage,
};
use tempfile::TempDir;

const TENANT: &str = "tenant";
const REPO: &str = "repo";
const BRANCH: &str = "main";
const WS: &str = "raisin:system";
const TOKEN_TYPE: &str = "raisin:OneTimeToken";

/// A storage with `raisin:system` and `raisin:OneTimeToken` seeded from the
/// shipped definitions.
///
/// Deliberately not hand-written fixtures: the point is to exercise the store
/// against the same YAML production loads, so that dropping the `index` on
/// `token_prefix` — which would silently turn every lookup into a miss — is
/// caught here.
async fn fixture(repos: &[&str]) -> Result<(TempDir, Arc<RocksDBStorage>)> {
    let temp_dir = TempDir::new().unwrap();
    let mut config = RocksDBConfig::default();
    config.path = temp_dir.path().to_path_buf();
    let storage = Arc::new(RocksDBStorage::with_config(config)?);

    storage
        .registry()
        .register_tenant(TENANT, HashMap::new())
        .await?;

    let system_ws = load_global_workspaces()
        .into_iter()
        .find(|w| w.name == WS)
        .expect("the shipped global workspaces must include raisin:system");

    // The node types are NOT seeded here: creating a repository already installs
    // the whole embedded global set, `raisin:OneTimeToken` included. Registering
    // them again fails with AlreadyExists. Asserted below rather than assumed —
    // if repo creation ever stops installing them, this fixture should fail
    // loudly rather than the tests failing obscurely on a missing type.
    assert!(
        load_embedded_nodetypes_with_hashes()
            .iter()
            .any(|(t, _)| t.name == TOKEN_TYPE),
        "raisin:OneTimeToken must ship as a global nodetype"
    );

    for repo in repos {
        storage
            .repository_management()
            .create_repository(
                TENANT,
                repo,
                RepositoryConfig {
                    default_language: "en".to_string(),
                    supported_languages: vec!["en".to_string()],
                    locale_fallback_chains: HashMap::new(),
                    default_branch: BRANCH.to_string(),
                    description: None,
                    tags: HashMap::new(),
                },
            )
            .await?;
        storage
            .branches()
            .create_branch(TENANT, repo, BRANCH, "system", None, None, false, false)
            .await?;

        let mut ws = system_ws.clone();
        ws.config.default_branch = BRANCH.to_string();
        WorkspaceService::new(storage.clone())
            .put(TENANT, repo, ws)
            .await?;
    }

    Ok((temp_dir, storage))
}

/// A token that expires `minutes` from now. `minutes` may be negative.
fn token(id: &str, hash: &str, minutes: i64) -> OneTimeToken {
    let now = StorageTimestamp::now();
    OneTimeToken {
        token_id: id.to_string(),
        tenant_id: TENANT.to_string(),
        token_hash: hash.to_string(),
        // The first 8 characters, matching how the auth strategy derives it.
        token_prefix: hash.chars().take(8).collect(),
        purpose: TokenPurpose::MagicLink,
        identity_id: None,
        email: "buyer@example.com".to_string(),
        created_at: now,
        expires_at: StorageTimestamp::from_nanos(
            now.timestamp_nanos() + minutes * 60 * 1_000_000_000,
        )
        .expect("a token expiry must be representable"),
        used: false,
        used_at: None,
        context: HashMap::new(),
    }
}

fn store(storage: &Arc<RocksDBStorage>, repo: &str) -> OneTimeTokenStore {
    OneTimeTokenStore::new(storage.clone(), TENANT, repo)
}

/// The whole happy path, end to end against real storage: a token is written,
/// found by its indexed prefix, matched on its hash, and consumed.
#[tokio::test]
async fn a_token_round_trips_and_is_found_by_its_indexed_prefix() -> Result<()> {
    let (_dir, storage) = fixture(&[REPO]).await?;
    let s = store(&storage, REPO);

    let t = token("tok-1", "abcdef0123456789", 15);
    s.create(&t).await?;

    let found = s
        .find_by_token(&t.token_prefix, &t.token_hash)
        .await?
        .expect("a freshly created token must be findable by prefix + hash");

    assert_eq!(found.token_id, "tok-1");
    assert_eq!(found.email, "buyer@example.com");
    assert!(!found.used);
    assert!(found.is_valid(), "a fresh, unused token must be valid");

    Ok(())
}

/// Replay protection. This is the one that matters most: if `mark_used` does not
/// durably land, a magic link stays redeemable for its whole lifetime and the
/// "one-time" in the name is decoration.
#[tokio::test]
async fn a_consumed_token_cannot_be_redeemed_twice() -> Result<()> {
    let (_dir, storage) = fixture(&[REPO]).await?;
    let s = store(&storage, REPO);

    let t = token("tok-replay", "cafebabe00000000", 15);
    s.create(&t).await?;

    assert!(s.mark_used("tok-replay").await?, "first consume succeeds");

    // Re-read through the same path a second redemption would take.
    let found = s
        .find_by_token(&t.token_prefix, &t.token_hash)
        .await?
        .expect("the record survives being consumed — it is the audit trail");

    assert!(found.used, "the used flag must have persisted");
    assert!(
        found.used_at.is_some(),
        "consuming must stamp when it happened"
    );
    assert!(
        !found.is_valid(),
        "a consumed token must not validate, or the link is replayable"
    );

    Ok(())
}

/// An expired token is still stored, still findable, and NOT valid. Lookup and
/// validity are deliberately separate — see the store's module docs.
#[tokio::test]
async fn an_expired_token_is_found_but_refused() -> Result<()> {
    let (_dir, storage) = fixture(&[REPO]).await?;
    let s = store(&storage, REPO);

    let t = token("tok-old", "deadbeef00000000", -1);
    s.create(&t).await?;

    let found = s
        .find_by_token(&t.token_prefix, &t.token_hash)
        .await?
        .expect("expiry must not make the record unfindable");

    assert!(!found.is_valid(), "an expired token must not validate");

    Ok(())
}

/// A wrong hash sharing a prefix must not match.
///
/// The prefix is only a narrowing device; it is the hash comparison that decides.
/// If a caller could redeem on prefix alone, guessing an 8-character prefix would
/// be enough to steal a session.
#[tokio::test]
async fn a_prefix_collision_does_not_authenticate() -> Result<()> {
    let (_dir, storage) = fixture(&[REPO]).await?;
    let s = store(&storage, REPO);

    let real = token("tok-real", "aaaaaaaa1111111111", 15);
    s.create(&real).await?;

    // Same prefix, different hash — exactly what an attacker who guessed the
    // visible part of a token would present.
    let guessed_hash = "aaaaaaaa9999999999";
    assert_eq!(
        &guessed_hash[..8],
        &real.token_hash[..8],
        "prefixes collide"
    );

    assert!(
        s.find_by_token(&real.token_prefix, guessed_hash)
            .await?
            .is_none(),
        "a matching prefix with a wrong hash must not resolve to the real token"
    );

    // And the real hash still works, so the collision did not poison the lookup.
    assert!(s
        .find_by_token(&real.token_prefix, &real.token_hash)
        .await?
        .is_some());

    Ok(())
}

/// Tokens are per repo. A link minted in one repo must not be redeemable in
/// another — this is what the tenant-wide endpoint's repo resolution protects.
#[tokio::test]
async fn a_token_is_not_visible_from_another_repo() -> Result<()> {
    let (_dir, storage) = fixture(&[REPO, "other"]).await?;

    let t = token("tok-scoped", "11112222abcdef9900", 15);
    store(&storage, REPO).create(&t).await?;

    assert!(
        store(&storage, REPO)
            .find_by_token(&t.token_prefix, &t.token_hash)
            .await?
            .is_some(),
        "found in the repo it was minted in"
    );
    assert!(
        store(&storage, "other")
            .find_by_token(&t.token_prefix, &t.token_hash)
            .await?
            .is_none(),
        "a token minted in one repo must not resolve in another"
    );

    Ok(())
}

/// Pruning removes what has expired and keeps what has not.
///
/// Also pins the deliberate choice that a USED but unexpired token survives: it
/// is the only record that the link was consumed, and its remaining minutes are
/// worth keeping for that.
#[tokio::test]
async fn pruning_removes_expired_and_spares_the_rest() -> Result<()> {
    let (_dir, storage) = fixture(&[REPO]).await?;
    let s = store(&storage, REPO);

    let live = token("tok-live", "1111aaaa22223333", 15);
    let stale = token("tok-stale", "4444bbbb55556666", -30);
    let used_but_live = token("tok-used", "7777cccc88889999", 15);

    s.create(&live).await?;
    s.create(&stale).await?;
    s.create(&used_but_live).await?;
    s.mark_used("tok-used").await?;

    let pruned = s.prune_expired().await?;
    assert_eq!(pruned, 1, "exactly the expired one goes");

    assert!(
        s.find_by_token(&live.token_prefix, &live.token_hash)
            .await?
            .is_some(),
        "a live token survives"
    );
    assert!(
        s.find_by_token(&stale.token_prefix, &stale.token_hash)
            .await?
            .is_none(),
        "an expired token is gone"
    );
    assert!(
        s.find_by_token(&used_but_live.token_prefix, &used_but_live.token_hash)
            .await?
            .is_some(),
        "a consumed but unexpired token is kept as the audit trail"
    );

    // Idempotent: a second sweep with nothing left to do removes nothing.
    assert_eq!(s.prune_expired().await?, 0);

    Ok(())
}

/// `mark_used` on a token that is gone reports false rather than erroring.
///
/// The caller must fail closed on an ERROR, so the two outcomes have to stay
/// distinguishable: "already pruned" is not the same as "the store is broken".
#[tokio::test]
async fn marking_a_missing_token_reports_false_rather_than_failing() -> Result<()> {
    let (_dir, storage) = fixture(&[REPO]).await?;
    let s = store(&storage, REPO);

    assert!(
        !s.mark_used("never-existed").await?,
        "a missing token is a false, not an Err"
    );

    Ok(())
}

/// An all-numeric token prefix must be findable.
///
/// Not a hypothetical: a prefix is the first 8 characters of
/// `hex::encode(32 random bytes)`, so it is all digits with probability
/// (10/16)^8 — about one magic link in 43. Before the storage sentinel this
/// missed every time, and the failure was invisible: the index entry was written
/// correctly and the same node stayed findable by every other property, so a
/// valid unexpired link simply reported itself invalid.
#[tokio::test]
async fn an_all_numeric_prefix_is_still_findable() -> Result<()> {
    let (_dir, storage) = fixture(&[REPO]).await?;
    let s = store(&storage, REPO);

    let t = token("tok-numeric", "11112222abcdef9900", 15);
    assert!(
        t.token_prefix.chars().all(|c| c.is_ascii_digit()),
        "this test is meaningless unless the prefix really is all digits"
    );
    s.create(&t).await?;

    let found = s
        .find_by_token(&t.token_prefix, &t.token_hash)
        .await?
        .expect("an all-numeric prefix must resolve like any other");

    assert_eq!(found.token_id, "tok-numeric");
    // The sentinel is a storage detail and must not leak back to the caller.
    assert_eq!(
        found.token_prefix, t.token_prefix,
        "the prefix must round-trip without the storage sentinel"
    );

    Ok(())
}
