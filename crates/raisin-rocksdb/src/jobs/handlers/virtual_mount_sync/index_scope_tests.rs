//! What the sync index READS, and what it must still SEE.
//!
//! A child module of [`super::tests`] (declared with `#[path]` at the bottom of
//! `tests.rs`) so it can reuse that file's environment and helpers.
//!
//! Two things are being pinned here and they pull in opposite directions:
//!
//! * the loader must read the mount's slice of the workspace and NOT the rest
//!   of it — that is the whole cost of a sync run's first phase; and
//! * the index it builds must still contain every node under the mount path,
//!   **foreign ones included**, because the guard that stops a mount clobbering
//!   user content is a `by_path` lookup on nodes `by_external` deliberately
//!   excludes.
//!
//! A narrowing that satisfies the first and breaks the second is silent data
//! loss, so neither test is meaningful without the other.

use raisin_models::nodes::Node;

use super::*;

/// Nodes outside the mount, in their own subtree.
async fn seed_outside(env: &Env, n: usize) {
    let tx = begin(env).await;
    for i in 0..n {
        tx.add_node(
            TARGET_WS,
            &Node {
                id: nanoid::nanoid!(),
                node_type: "raisin:Node".to_string(),
                name: format!("other{i}"),
                path: format!("/other/other{i}"),
                workspace: Some(TARGET_WS.to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();
}

/// Write one node at an absolute path, creating ancestors as needed.
async fn put_at(env: &Env, path: &str, props: &[(&str, &str)]) {
    let tx = begin(env).await;
    let mut node = Node {
        id: nanoid::nanoid!(),
        node_type: "raisin:Node".to_string(),
        name: path.rsplit('/').next().unwrap().to_string(),
        path: path.to_string(),
        workspace: Some(TARGET_WS.to_string()),
        ..Default::default()
    };
    for (k, v) in props {
        node.properties
            .insert((*k).to_string(), PropertyValue::String((*v).to_string()));
    }
    tx.upsert_deep_node(TARGET_WS, &node, "raisin:Folder")
        .await
        .unwrap();
    tx.commit().await.unwrap();
}

/// The read the index load actually performs is bounded by the mount path.
///
/// This is the measurement, not a proxy for it: `load_index_nodes` returns the
/// nodes it materialised out of RocksDB, so its length IS the number of node
/// blobs deserialised — the dominant cost of a run's first phase, paid once per
/// mount per tick.
///
/// Before the path-prefix scan this returned the WHOLE workspace (the mount
/// filter ran afterwards, in memory), so the assertion below read 203 instead of
/// 3 and the cost of every mount's sync scaled with content it does not own.
#[tokio::test(flavor = "multi_thread")]
async fn the_index_load_reads_only_the_mount_subtree() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());

    seed_outside(&env, 200).await;
    put_at(&env, "/drive/a.txt", &[]).await;
    put_at(&env, "/drive/b.txt", &[]).await;

    let read = mat.load_index_nodes(&scope()).await.unwrap();
    let paths: Vec<&str> = read.iter().map(|n| n.path.as_str()).collect();

    // `/drive` itself plus its two children. Nothing from `/other`.
    assert_eq!(
        read.len(),
        3,
        "the loader must read the mount subtree only, got {paths:?}"
    );
    assert!(paths.contains(&"/drive"), "{paths:?}");
    assert!(paths.contains(&"/drive/a.txt"), "{paths:?}");
    assert!(paths.contains(&"/drive/b.txt"), "{paths:?}");
    assert!(
        !paths.iter().any(|p| p.starts_with("/other")),
        "unrelated content must not be read: {paths:?}"
    );
}

/// The narrowing must not change what the index CONTAINS.
///
/// Three properties in one test because they fail as one bug — a scan narrowed
/// by ownership, or by an exact-string prefix, breaks whichever of them it
/// forgets:
///
/// * a FOREIGN node under the mount path is present in `by_path`. This is the
///   never-overwrite-user-content guard's only input; drop it and the mount
///   silently replaces a user's file.
/// * the mount root node itself is present. A mapper that resolves an item to
///   the empty relative path targets exactly `mount_path`, and a strict
///   descendant scan would report that path free.
/// * a sibling sharing a textual prefix (`/driveways`) is NOT present. A key
///   prefix is a byte prefix, so the scan does return it; `under()` is what
///   rejects it, and it must keep doing so.
#[tokio::test(flavor = "multi_thread")]
async fn the_index_still_sees_foreign_nodes_and_not_prefix_siblings() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());

    put_at(&env, "/drive/mine.txt", &[]).await;
    put_at(
        &env,
        "/drive/synced.txt",
        &[
            ("__mount_id", MOUNT_ID),
            ("__external_id", "X"),
            ("__etag", "v1"),
        ],
    )
    .await;
    put_at(&env, "/driveways/decoy.txt", &[]).await;
    seed_outside(&env, 5).await;

    let idx = mat.load_index(&scope()).await.unwrap();

    assert_eq!(
        idx.at_path("/drive/mine.txt").map(|e| e.mount_owned),
        Some(false),
        "a foreign node under the mount must stay visible by path"
    );
    assert_eq!(
        idx.at_path("/drive/synced.txt").map(|e| e.mount_owned),
        Some(true)
    );
    assert!(
        idx.at_path("/drive").is_some(),
        "the mount root itself occupies a path"
    );
    assert!(
        idx.at_path("/driveways/decoy.txt").is_none(),
        "a textual-prefix sibling is not under the mount"
    );
    assert!(idx.at_path("/other/other0").is_none());

    assert_eq!(idx.virtual_len(), 1);
    assert_eq!(idx.etag_for("X"), Some("v1"));
}

/// A mount rooted at `/` owns the whole workspace, and must still see all of it.
///
/// The prefix `/` is a prefix of every path, so this needs no special case — but
/// it is exactly the case a "strict descendants of the mount path" narrowing
/// would get wrong, and the one where the old and new loaders must agree
/// completely.
#[tokio::test(flavor = "multi_thread")]
async fn a_root_mount_still_sees_the_whole_workspace() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());

    seed_outside(&env, 20).await;
    put_at(&env, "/drive/a.txt", &[]).await;

    let root_scope = MountScope {
        mount_path: "/".to_string(),
        ..scope()
    };
    let read = mat.load_index_nodes(&root_scope).await.unwrap();
    let idx = mat.load_index(&root_scope).await.unwrap();

    // 20 under /other (`add_node` creates no ancestor folder), plus /drive and
    // /drive/a.txt.
    assert_eq!(read.len(), 22, "a root mount reads everything");
    assert!(idx.at_path("/other/other0").is_some());
    assert!(idx.at_path("/drive/a.txt").is_some());
}

/// The cost of a run's first phase must not scale with content the mount does
/// not own.
///
/// Ignored, and it ASSERTS only the invariant (unrelated content stays out of
/// the index); the two wall-clock numbers are PRINTED, not thresholded, because
/// a timing budget here would be a machine-speed flake. Run it before and after
/// the change to see the difference the printout is there for.
///
/// `cargo test -p raisin-rocksdb --lib index_scope_tests::index_load_cost -- --ignored --nocapture`
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn index_load_cost_is_flat_in_unrelated_content() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    for i in 0..200 {
        put_at(&env, &format!("/drive/m{i}.txt"), &[]).await;
    }

    let t0 = std::time::Instant::now();
    let small = mat.load_index_nodes(&scope()).await.unwrap().len();
    let small_elapsed = t0.elapsed();

    seed_outside(&env, 20_000).await;

    let t1 = std::time::Instant::now();
    let big = mat.load_index_nodes(&scope()).await.unwrap().len();
    let big_elapsed = t1.elapsed();

    println!(
        "index load: {small} nodes in {small_elapsed:?}; \
         after +20000 unrelated nodes: {big} nodes in {big_elapsed:?}"
    );
    assert_eq!(small, big, "unrelated content must not enter the index");
}
