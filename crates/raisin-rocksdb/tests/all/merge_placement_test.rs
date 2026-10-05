//! Plan Phase 13a review: the merge-resolution funnel had the same
//! move-out / move-in hazard 13a fixed for promotion. A resolution ends each
//! superseded version's old path and UNIQUE claims at the merge revision M,
//! and those keys carry no node id — so the tombstone landed on whatever else
//! held the path or value at M (another resolution of the same merge, each its
//! own commit at the SAME M), and masked everything below M: a node the target
//! created at the vacated path, and the source entries `copy_branch_indexes`
//! replays at their original revisions AFTER the resolutions. The path then
//! resolved to nothing and the value had no claim while a node held it.
//!
//! A resolution now ends an old path only while the MERGED view (target at or
//! before M, source at or before its HEAD) still gives it to the node, and
//! leaves a claim the merged view gives to another node to it
//! (`merge::merged_view`), in the resolution and in its commit-time
//! correction.

use crate::merge_apply_funnel_test::{node, Env, REPO, TENANT, WS};
use crate::promotion_unique_test::{account_type, ACCOUNT, EMAIL};
use raisin_context::{ConflictResolution, MergeStrategy, ResolutionType};
use raisin_error::Result;
use raisin_models::nodes::Node;
use raisin_rocksdb::{cf, keys};
use raisin_storage::{BranchScope, CommitMetadata, NodeTypeRepository, Storage};

fn account(id: &str, path: &str, email: &str, title: &str) -> Node {
    let mut n = node(id, path, &[(EMAIL, email), ("title", title)]);
    n.node_type = ACCOUNT.to_string();
    n
}

async fn with_account_type(env: &Env) -> Result<()> {
    env.storage
        .node_types()
        .upsert(
            BranchScope::new(TENANT, REPO, "main"),
            account_type(),
            CommitMetadata::system("account type"),
        )
        .await?;
    Ok(())
}

/// Detect the conflict, then resolve `ids` keep-theirs, in that order.
async fn keep_theirs(env: &Env, ids: &[&str]) -> Result<()> {
    let branches = env.storage.branches_impl();
    let attempt = branches
        .merge_branches(
            TENANT,
            REPO,
            "main",
            "feature",
            MergeStrategy::ThreeWay,
            "attempt",
            "test-user",
        )
        .await?;
    assert!(!attempt.conflicts.is_empty(), "the branches must conflict");
    let resolutions = ids
        .iter()
        .map(|id| ConflictResolution {
            node_id: id.to_string(),
            resolution_type: ResolutionType::KeepTheirs,
            resolved_properties: serde_json::Value::Null,
            translation_locale: None,
        })
        .collect();
    let result = branches
        .resolve_merge_with_resolutions(
            TENANT,
            REPO,
            "main",
            "feature",
            resolutions,
            "resolved",
            "test-user",
        )
        .await?;
    assert!(result.success);
    Ok(())
}

/// The owner the newest RAW entry under `prefix` in `cf_name` names
/// (`None`: no entry, or a tombstone).
fn newest_owner(env: &Env, cf_name: &str, prefix: &[u8]) -> Option<String> {
    let db = env.storage.db();
    let cf = db.cf_handle(cf_name).unwrap();
    let (key, value) = db.prefix_iterator_cf(cf, prefix).next()?.unwrap();
    if !key.starts_with(prefix) || keys::is_tombstone_value(&value) {
        return None;
    }
    Some(String::from_utf8(value.to_vec()).unwrap())
}

fn path_owner(env: &Env, path: &str) -> Option<String> {
    let prefix = keys::path_index_key_prefix(TENANT, REPO, "main", WS, path);
    newest_owner(env, cf::PATH_INDEX, &prefix)
}

fn claim_owner(env: &Env, email: &str) -> Option<String> {
    let prefix = keys::unique_index_value_prefix(TENANT, REPO, "main", WS, ACCOUNT, EMAIL, email);
    newest_owner(env, cf::UNIQUE_INDEX, &prefix)
}

#[tokio::test]
async fn a_resolution_keeps_the_path_and_claim_a_source_node_took_from_it() -> Result<()> {
    let env = Env::new().await?;
    with_account_type(&env).await?;
    env.add("main", account("a", "/p", "v@x", "base")).await?;
    env.fork().await?;
    env.put("main", account("a", "/p", "v@x", "ours")).await?;
    // On the source A moves away and gives its value up; B takes both.
    env.put("feature", account("a", "/q", "w@x", "theirs"))
        .await?;
    env.add("feature", account("b", "/p", "v@x", "new")).await?;

    keep_theirs(&env, &["a"]).await?;

    assert_eq!(env.id_at("/p").await.as_deref(), Some("b"));
    assert_eq!(path_owner(&env, "/p").as_deref(), Some("b"), "raw /p");
    assert_eq!(env.id_at("/q").await.as_deref(), Some("a"));
    assert_eq!(claim_owner(&env, "v@x").as_deref(), Some("b"), "claim v");
    assert_eq!(claim_owner(&env, "w@x").as_deref(), Some("a"), "claim w");
    Ok(())
}

#[tokio::test]
async fn two_resolutions_at_one_merge_revision_keep_the_path_in_both_orders() -> Result<()> {
    for order in [["a", "b"], ["b", "a"]] {
        let env = Env::new().await?;
        env.add("main", node("a", "/p", &[("title", "base")]))
            .await?;
        env.add("main", node("b", "/x", &[("title", "base")]))
            .await?;
        env.fork().await?;
        env.put("main", node("a", "/p", &[("title", "ours")]))
            .await?;
        env.put("main", node("b", "/x", &[("title", "ours")]))
            .await?;
        // On the source A moves out of /p and B moves into it.
        env.put("feature", node("a", "/q", &[("title", "theirs")]))
            .await?;
        env.put("feature", node("b", "/p", &[("title", "theirs")]))
            .await?;

        keep_theirs(&env, &order).await?;

        assert_eq!(path_owner(&env, "/p").as_deref(), Some("b"), "{order:?}");
        assert_eq!(path_owner(&env, "/q").as_deref(), Some("a"), "{order:?}");
        assert_eq!(path_owner(&env, "/x"), None, "{order:?}");
    }
    Ok(())
}

#[tokio::test]
async fn a_resolution_leaves_a_node_the_target_created_at_its_base_path() -> Result<()> {
    let env = Env::new().await?;
    env.add("main", node("a", "/p", &[("title", "base")]))
        .await?;
    env.fork().await?;
    env.put("feature", node("a", "/p", &[("title", "theirs")]))
        .await?;
    // On the target A moves away and C is created where it was.
    env.put("main", node("a", "/q", &[("title", "ours")]))
        .await?;
    env.add("main", node("c", "/p", &[("title", "new")]))
        .await?;

    env.conflict_and_resolve("a", ResolutionType::KeepOurs)
        .await?;

    assert_eq!(env.id_at("/p").await.as_deref(), Some("c"));
    assert_eq!(path_owner(&env, "/p").as_deref(), Some("c"), "raw /p");
    assert_eq!(path_owner(&env, "/q").as_deref(), Some("a"), "raw /q");
    Ok(())
}
