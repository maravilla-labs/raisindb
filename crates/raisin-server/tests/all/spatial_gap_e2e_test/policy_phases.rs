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

//! Gap 3: a precision-policy change reaches subsequent writes, queues a real
//! rebuild, and never makes a query answer partially while it lands.

use super::fixture::insert_via_sql;
use super::observe::{
    await_health, health, job_info, list_jobs, put_config, sorted, verify, Health,
};
use super::queries::Baseline;
use super::transport::{rows, sql, DEFAULT_PRECISIONS, WORKSPACE};

/// Filler rows, far from either query centre, so the rebuild has enough work to
/// spend a measurable amount of time in `Building`.
pub const FILLERS: usize = 20;

/// After the HTTP config change. Drops five default precisions and adds 5, so the
/// rebuild has to REMOVE entries as well as add them — a gap-fill would not.
const HTTP_SET: [usize; 5] = [11, 9, 7, 5, 2];
/// After the SQL `ALTER`. Adds 3.
const SQL_SET: [usize; 6] = [11, 9, 7, 5, 3, 2];
/// Deliberately DISJOINT from `SQL_SET`, so mid-rebuild no precision is complete
/// and the index must declare itself unusable rather than answer from half of it.
pub const DISJOINT_SET: [usize; 5] = [10, 8, 6, 4, 1];

/// A projected probe row, written while a policy change is in flight — so this
/// keeps exercising gap 1 as well.
const PROBE_3857: (f64, f64) = (-3_896_182.177_222_222, 3_503_549.843_016_676);
/// The same place in WGS84, for the second probe.
const PROBE_WGS84: (f64, f64) = (-35.0, 30.0);

/// Insert `FILLERS` rows in the middle of the Atlantic — outside every query
/// radius, so the baselines stay fixed while the rebuild gets real work to do.
pub async fn padding_rows(base_url: &str, token: &str, baseline: &Baseline) {
    println!("\n--- 4. padding the workspace with {FILLERS} far-away rows ---");
    for i in 0..FILLERS {
        let lon = -40.0 + i as f64 * 0.5;
        insert_via_sql(base_url, token, &format!("filler-{i:02}"), lon, 30.0, None)
            .await
            .unwrap_or_else(|e| panic!("filler {i} insert failed: {e}"));
    }
    let want = (4 + FILLERS) as u64;
    await_health(base_url, token, "filler rows indexed", |h| {
        h.distinct_nodes == want && h.live_entries == want * DEFAULT_PRECISIONS.len() as u64
    })
    .await;
    baseline
        .assert_unchanged(base_url, token, "after padding")
        .await;
}

/// Phase 5 — `PUT …/spatial/config` reaches subsequent writes AND queues a rebuild.
///
/// Both halves are asserted against something real: the rebuild is looked up in
/// the job queue by the id the endpoint returned (a 2xx proves nothing), and the
/// write side is asserted on the physical entry census, at a precision the old
/// policy never emitted.
pub async fn http_config_change_reaches_writes_and_queues_a_rebuild(
    base_url: &str,
    token: &str,
    baseline: &Baseline,
) {
    println!("\n--- 5. PUT spatial/config: writes pick it up, a rebuild is queued ---");
    let before = health(base_url, token).await;
    assert_eq!(before.at(5), 0, "precision 5 must be absent to start with");

    let response = put_config(base_url, token, &HTTP_SET).await;
    println!("[put config] {response}");
    assert_eq!(
        sorted(
            response["precisions"]
                .as_array()
                .expect("precisions")
                .iter()
                .filter_map(|v| v.as_u64().map(|n| n as usize))
                .collect()
        ),
        sorted(HTTP_SET.to_vec())
    );

    // -- the rebuild is real: find it in the job queue, by id. ---------------
    let job_id = response["rebuild_job_id"]
        .as_str()
        .expect(
            "a policy change that drifts from the local index must queue a rebuild; \
             without one a read-mostly workspace waits forever",
        )
        .to_string();
    let job = job_info(base_url, token, &job_id)
        .await
        .unwrap_or_else(|e| panic!("queued rebuild {job_id} is not in the job queue: {e}"));
    println!("[queued job] {job}");
    let job_type = job["job_type"].as_str().unwrap_or_default();
    assert!(
        job_type.contains("SpatialIndexBuild"),
        "the queued job must be a spatial build, got {job_type}"
    );
    assert!(
        job_type.ends_with("/true)"),
        "a policy change must queue a REBUILD (tombstone-then-re-emit), not a \
         gap-fill: a shrinking precision set has to remove entries. Got {job_type}"
    );
    assert!(
        list_jobs(base_url, token)
            .await
            .iter()
            .any(|j| j["id"].as_str() == Some(job_id.as_str())),
        "the rebuild must also be visible in the job listing"
    );
    println!("[PASS] rebuild {job_id} queued and visible in the queue");

    // -- the write side: a row written now carries the newly configured set. -
    insert_via_sql(
        base_url,
        token,
        "probe-mercator",
        PROBE_3857.0,
        PROBE_3857.1,
        Some(3857),
    )
    .await
    .expect("probe insert (EPSG:3857) during the policy window");
    assert_write_adopted_precision(base_url, token, 5, &before).await;

    let want = (5 + FILLERS) as u64;
    let h = await_health(base_url, token, "rebuild settles at the HTTP set", |h| {
        h.settled_at(&HTTP_SET) && h.distinct_nodes == want
    })
    .await;
    assert_eq!(
        h.at(5),
        want,
        "every row must end up at the new precision: {h}"
    );
    assert_eq!(
        h.at(10),
        0,
        "precision 10 was dropped and must be gone: {h}"
    );
    let (status, detail) = verify(base_url, token).await;
    assert_eq!(status, "OK", "VERIFY after the rebuild: {detail}");
    baseline
        .assert_unchanged(base_url, token, "after the HTTP policy change")
        .await;
    println!("[PASS] policy change landed on every row; answers unchanged");
}

/// Phase 6 — SQL `ALTER` moves intent only; the next write moves reality.
///
/// The SQL surface deliberately does NOT queue a rebuild (the operator asks for
/// one explicitly with `REBUILD SPATIAL INDEX`), which makes it the clean way to
/// observe the drift window: intent has moved, nothing has rewritten anything,
/// and query results must be identical anyway.
pub async fn sql_config_change_alone_moves_nothing(
    base_url: &str,
    token: &str,
    baseline: &Baseline,
) {
    println!("\n--- 6. SQL ALTER: intent moves, reality does not, answers hold ---");
    let before = health(base_url, token).await;

    let list = SQL_SET
        .iter()
        .map(|p| p.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    sql(
        base_url,
        token,
        &format!("ALTER SPATIAL INDEX FOR '{WORKSPACE}' PROPERTY 'geom' SET PRECISIONS = ({list})"),
    )
    .await
    .expect("ALTER SPATIAL INDEX");

    let shown = sql(
        base_url,
        token,
        &format!("SHOW SPATIAL INDEX CONFIG FOR '{WORKSPACE}' PROPERTY 'geom'"),
    )
    .await
    .expect("SHOW SPATIAL INDEX CONFIG");
    assert_eq!(
        rows(&shown)[0]["precisions"].as_str().unwrap(),
        format!("({list})"),
        "the configured policy must reflect the ALTER"
    );

    // Drift is visible on BOTH admin surfaces, and nothing has been rewritten.
    let drifted = health(base_url, token).await;
    assert_eq!(
        drifted.indexed, before.indexed,
        "ALTER must not rewrite the index"
    );
    assert_eq!(drifted.configured, sorted(SQL_SET.to_vec()));
    assert!(
        drifted.needs_rebuild,
        "the drift must be visible: {drifted}"
    );
    let (status, detail) = verify(base_url, token).await;
    assert_eq!(
        status, "NEEDS_REBUILD",
        "VERIFY must report the drift it can see: {detail}"
    );
    assert!(
        detail.contains("policy drift"),
        "VERIFY must say what drifted: {detail}"
    );

    // Give any (unexpected) reconciliation a chance to fire, then prove nothing did.
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    let still = health(base_url, token).await;
    assert_eq!(still.live_entries, before.live_entries, "{still}");
    assert_eq!(still.at(3), 0, "nothing may write precision 3 yet: {still}");
    baseline
        .assert_unchanged(base_url, token, "while intent and reality disagree")
        .await;
    println!("[PASS] config change alone rewrote nothing and changed no answer");

    // The next write is what moves reality — and it must use CONFIGURED intent.
    insert_via_sql(
        base_url,
        token,
        "probe-wgs84",
        PROBE_WGS84.0,
        PROBE_WGS84.1,
        None,
    )
    .await
    .expect("probe insert during the drift window");
    assert_write_adopted_precision(base_url, token, 3, &before).await;

    let want = (6 + FILLERS) as u64;
    await_health(
        base_url,
        token,
        "reconciliation settles at the SQL set",
        |h| h.settled_at(&SQL_SET) && h.distinct_nodes == want,
    )
    .await;
    let (status, detail) = verify(base_url, token).await;
    assert_eq!(status, "OK", "VERIFY after reconciliation: {detail}");
    baseline
        .assert_unchanged(base_url, token, "after reconciliation")
        .await;
    println!("[PASS] the write adopted configured intent and reconciliation followed");
}

/// Assert that a row written under a changed policy carries `precision`.
///
/// Attribution matters here: the entries could also come from the rebuild that a
/// policy change queues. When the state record still shows the OLD precisions and
/// the phase is `ready`, no rebuild has run, so a single entry at the new
/// precision can only have come from the write itself — that case is asserted
/// exactly. Otherwise the weaker (still regression-proving) form is used: before
/// the fix, a configured precision reached the entries by NO route at all.
async fn assert_write_adopted_precision(
    base_url: &str,
    token: &str,
    precision: usize,
    before: &Health,
) {
    let h = health(base_url, token).await;
    assert!(
        h.at(precision) >= 1,
        "a write under the new policy must emit precision {precision}; before the \
         fix the write path resolved its precisions from the local state record, so \
         a configured change reached the entries by no route at all: {h}"
    );
    if h.phase == "ready" && h.indexed == before.indexed {
        assert_eq!(
            h.at(precision),
            1,
            "no rebuild has run yet (phase=ready, state record unchanged), so the \
             single new row is the only possible source of precision {precision}: {h}"
        );
        println!("[PASS] precision {precision} came from the WRITE (no rebuild had run yet): {h}");
    } else {
        println!(
            "[PASS] precision {precision} present ({} entries); the queued rebuild had \
             already started, so this observation does not isolate the write path: {h}",
            h.at(precision)
        );
    }
}
