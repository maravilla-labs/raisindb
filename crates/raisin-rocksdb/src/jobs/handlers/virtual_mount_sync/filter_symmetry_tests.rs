//! One item, one answer: the full walk and the delta must filter identically.
//!
//! `exclude: ["Archive"]` reads to an operator as "do not sync the Archive
//! folder". The delta computed an item's whole path and then tested only the
//! LEAF, so `Archive/x.pdf` — which the glob `Archive` does not match — was
//! admitted, and the excluded folder's entire contents synced anyway.
//!
//! A delta/walk disagreement is the class of bug that has already cost us twice
//! (google-drive, ms-graph): the same item resolves one way on one path and
//! another on the other, and the engine relocates or re-imports the node on
//! every alternate run. Here it also imported data an operator had asked to be
//! left alone, which is why the assertions below run BOTH paths over the same
//! item and demand the same outcome.
//!
//! A child module of [`super::tests`] (declared with `#[path]` at the bottom of
//! `tests.rs`) so it can reuse that file's environment, mocks and helpers.

use serde_json::json;

use super::*;
use crate::jobs::handlers::virtual_mount_sync as sync;
use sync::config::MountState;

/// A mount that excludes the `Archive` FOLDER by name — the pattern an operator
/// actually writes. Not `Archive/**`: the point of the test is that the plain
/// folder name has to be enough, on both paths.
fn archiving_mount() -> sync::config::MountConfig {
    let mut sync_config = SyncConfig::default();
    sync_config.exclude_patterns = vec!["Archive".to_string()];
    mk_mount(sync_config)
}

/// The full walk: `Archive` is not staged, and neither is anything under it.
#[tokio::test(flavor = "multi_thread")]
async fn the_full_walk_excludes_a_file_inside_an_excluded_folder() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mock = MockAdapter::default();
    // root: the excluded folder plus one ordinary file; the folder holds x.pdf.
    mock.set_list(
        "root",
        json!({ "items": [
            ext_item("ARCH", "Archive", true, "e1"),
            ext_item("KEEP", "keep.pdf", false, "e2")
        ], "next_cursor": null }),
    );
    mock.set_list(
        "ARCH",
        json!({ "items": [ext_item("X", "x.pdf", false, "e3")], "next_cursor": null }),
    );

    let mount = archiving_mount();
    let mut state = MountState::default();
    sync::full::run(&ctx(&env, &mount, &mock, &mat), &mut state)
        .await
        .unwrap();

    let nodes = all_nodes(&env, TARGET_WS).await;
    let ids: Vec<String> = virtual_assets(&nodes)
        .iter()
        .filter_map(|n| str_prop(n, "__external_id"))
        .collect();
    assert_eq!(
        ids,
        vec!["KEEP".to_string()],
        "only the unexcluded file may be materialized; got {ids:?}"
    );
    // And the subtree was never paged out of the provider at all: descending
    // into a folder whose every item will be rejected spends the run's item
    // budget for nothing.
    assert_eq!(
        mock.op_count("list"),
        1,
        "the walk must not descend into an excluded folder"
    );
}

/// The delta: the SAME item, reported with its full relative path, must be
/// filtered the same way.
///
/// This is the half that was wrong. `passes_filters("Archive/x.pdf", …)` tested
/// the leaf path against `Archive`, which does not match, so the item was
/// admitted — by the very run that would never have reached it on a walk.
#[tokio::test(flavor = "multi_thread")]
async fn the_delta_excludes_the_same_file_inside_the_same_excluded_folder() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mock = MockAdapter::default();
    mock.push_changes(json!({ "items": [
        { "type": "created", "item": ext_item("X", "x.pdf", false, "e3"),
          "relative_path": "Archive/x.pdf" },
        { "type": "created", "item": ext_item("KEEP", "keep.pdf", false, "e2"),
          "relative_path": "keep.pdf" }
    ], "next_token": null }));

    let mount = archiving_mount();
    let mut state = MountState {
        last_sync_token: Some("t0".to_string()),
        ..Default::default()
    };
    sync::delta::run(&ctx(&env, &mount, &mock, &mat), &mut state)
        .await
        .unwrap();

    let nodes = all_nodes(&env, TARGET_WS).await;
    let ids: Vec<String> = virtual_assets(&nodes)
        .iter()
        .filter_map(|n| str_prop(n, "__external_id"))
        .collect();
    assert_eq!(
        ids,
        vec!["KEEP".to_string()],
        "the delta must give the walk's answer for the same item; got {ids:?}"
    );
}

/// Inclusion is NOT inherited, and must not become so by symmetry.
///
/// An include list names the items to keep (`*.pdf`); no folder matches one, so
/// applying the ancestor rule to includes as well would refuse every item on
/// every mount that uses one.
///
/// What the rule actually is, because the name "leaf" would be a lie: an
/// include pattern is matched against the WHOLE mount-relative path, and
/// `*.pdf` reaches `Reports/q1.pdf` only because `glob`'s default
/// `MatchOptions` leave `require_literal_separator` FALSE, so `*` crosses `/`.
/// The last assertion pins that dependency: a pattern with no wildcard does not
/// match a deeper path, so anyone who tightens `MatchOptions` — or swaps the
/// glob engine for one that anchors segments — breaks every include mount in
/// production and must fail here first.
#[test]
fn an_include_pattern_is_matched_against_the_whole_relative_path() {
    let include = vec!["*.pdf".to_string()];
    assert!(sync::passes_filters("Reports/q1.pdf", &include, &[]));
    assert!(!sync::passes_filters("Reports/q1.txt", &include, &[]));
    // The dependency, stated: `*` is what crosses the separator, not the
    // matcher walking to the leaf.
    assert!(!sync::passes_filters(
        "Reports/q1.pdf",
        &["q1.pdf".to_string()],
        &[]
    ));
    assert!(sync::passes_filters("q1.pdf", &["q1.pdf".to_string()], &[]));
}

/// The ancestor rule in isolation, including what it must NOT match.
#[test]
fn exclusion_is_inherited_by_every_descendant_and_nothing_else() {
    let exclude = vec!["Archive".to_string()];
    assert!(sync::excluded("Archive", &exclude));
    assert!(sync::excluded("Archive/x.pdf", &exclude));
    assert!(sync::excluded("Archive/2024/q1/x.pdf", &exclude));
    // A prefix that is not a whole segment is a different folder.
    assert!(!sync::excluded("Archived/x.pdf", &exclude));
    assert!(!sync::excluded("Reports/Archive.pdf", &exclude));
    // A leading or doubled slash must not produce an empty `""` prefix (which a
    // bare `*` would match, excluding everything); the real segments are still
    // tested.
    assert!(sync::excluded("/a//b", &["a".to_string()]));
    assert!(!sync::excluded("/a//b", &["c".to_string()]));
    // No patterns, nothing excluded.
    assert!(!sync::excluded("a/b", &[]));
}

/// Inheriting exclusion must be a strict SUPERSET of the leaf test it replaced.
///
/// The ancestor walk normalises the path — it skips empty segments — so a
/// `rel_path` a mapper emitted with a leading slash is rebuilt without one. A
/// pattern written to match that exact path would then match nothing, and the
/// filter would fail OPEN on a live mount that is currently excluding the item.
/// The whole path is therefore tested verbatim first.
#[test]
fn a_pattern_that_matched_the_whole_path_before_still_matches_it() {
    assert!(sync::excluded(
        "/Archive/x.pdf",
        &["/Archive/x.pdf".to_string()]
    ));
    assert!(sync::excluded("a//b", &["a//b".to_string()]));
    // And a pattern that matched nothing still matches nothing.
    assert!(!sync::excluded("/Archive/x.pdf", &["/Other/*".to_string()]));
}

/// THE RELEASE-BLOCKING HALF: exclusion must not delete.
///
/// Inheriting exclusion means a live mount stops LISTING an excluded folder —
/// the walk does not even descend into it. Every node already synced under it is
/// then "not seen this pass", which is precisely what `reconcile_deletes` reads
/// as "deleted upstream". Adding one `exclude` pattern to a mount that already
/// synced that folder would therefore silently delete its entire contents on the
/// next full walk, unbounded, with nobody having asked for a delete.
///
/// The semantics that fix it: an excluded subtree is OUT OF SCOPE. The mount
/// neither creates nodes there nor deletes them — "I do not manage this" is not
/// "destroy what is there". And the retention is REPORTED (one warn, plus
/// `retained_excluded` on the run's counters), because a reconcile that silently
/// leaves nodes behind is as unreadable as one that silently removes them.
#[tokio::test(flavor = "multi_thread")]
async fn a_node_under_a_newly_excluded_folder_survives_the_walk_that_stopped_listing_it() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());

    // Synced BEFORE the operator added `exclude: ["Archive"]` — the whole point:
    // the node is mount-owned, carries an `__external_id`, and lives under the
    // now-excluded folder.
    let virt = VirtualMeta {
        mount_id: MOUNT_ID.to_string(),
        external_id: "X".to_string(),
        etag: Some("e3".to_string()),
        synced_at: Utc::now().to_rfc3339(),
    };
    let mapped = sync::default_mapping(
        &serde_json::from_value(ext_item("X", "x.pdf", false, "e3")).unwrap(),
    );
    let mut index = mat.load_index(&scope()).await.unwrap();
    upsert_one(&mat, &scope(), &mut index, "Archive/x.pdf", mapped, virt).await;

    // A clean, complete, authoritative walk — none of the guards that already
    // skip reconciliation apply — that simply no longer mentions `Archive`.
    let mock = MockAdapter::default();
    mock.set_list(
        "root",
        json!({ "items": [ext_item("KEEP", "keep.pdf", false, "e2")], "next_cursor": null }),
    );

    let mount = archiving_mount();
    let c = ctx(&env, &mount, &mock, &mat);
    // `run_with` rather than `run` so the test can read the run's counters, which
    // is where the skip is reported to the console.
    let mut batcher = sync::batch::SyncBatcher::new(&c).await.unwrap();
    let mut state = MountState::default();
    sync::full::run_with(&c, &mut state, &mut batcher)
        .await
        .unwrap();

    let ids: Vec<String> = list_virtual(&mat, &scope())
        .await
        .into_iter()
        .map(|n| n.external_id)
        .collect();
    assert!(
        ids.iter().any(|i| i == "X"),
        "a node under a newly-excluded folder must survive a walk that no longer lists it; \
         got {ids:?}"
    );
    assert!(
        ids.iter().any(|i| i == "KEEP"),
        "the unexcluded part of the mount must still sync; got {ids:?}"
    );
    assert_eq!(
        batcher.stats().retained_excluded,
        1,
        "the run must report how many nodes it left behind, not skip them silently"
    );
    assert_eq!(
        batcher.stats().deleted,
        0,
        "nothing was gone upstream, so nothing may be deleted"
    );
}
