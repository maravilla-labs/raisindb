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

//! The three radius queries every phase re-asks, and their expected answers.
//!
//! The expected answers are fixed once, in [`Baseline::capture`], and asserted
//! identical after every configuration change and every rebuild. That is the
//! load-bearing invariant of the whole precision-policy design: **precision is a
//! performance knob and must never be a correctness knob**, before, during or
//! after a migration.

use super::transport::{names, sql, WORKSPACE};
use super::{BERN, ZURICH};

/// A radius query in WGS84 around `centre`, returning matched names sorted.
///
/// The centre is an unlabelled `ST_POINT`, i.e. WGS84 — the whole point of gap 1
/// is that rows stored in *other* frames must answer it correctly.
pub async fn within(base_url: &str, token: &str, centre: (f64, f64), radius_m: u64) -> Vec<String> {
    let statement = format!(
        "SELECT name FROM '{WORKSPACE}' \
         WHERE ST_DWITHIN(properties->>'geom', ST_POINT({}, {}), {radius_m}) \
         ORDER BY name",
        centre.0, centre.1
    );
    let r = sql(base_url, token, &statement)
        .await
        .unwrap_or_else(|e| panic!("ST_DWITHIN({radius_m} m) failed: {e}"));
    names(&r)
}

/// `EXPLAIN` for the same query, as text — diagnostics for a failed comparison.
pub async fn explain(base_url: &str, token: &str, centre: (f64, f64), radius_m: u64) -> String {
    let statement = format!(
        "EXPLAIN SELECT name FROM '{WORKSPACE}' \
         WHERE ST_DWITHIN(properties->>'geom', ST_POINT({}, {}), {radius_m}) \
         ORDER BY name",
        centre.0, centre.1
    );
    match sql(base_url, token, &statement).await {
        Ok(v) => serde_json::to_string_pretty(&v["rows"]).unwrap_or_default(),
        Err(e) => format!("EXPLAIN failed: {e}"),
    }
}

/// The three answers that must never change.
#[derive(Debug, Clone)]
pub struct Baseline {
    pub near_zurich: Vec<String>,
    pub near_bern: Vec<String>,
    pub wide: Vec<String>,
}

impl Baseline {
    /// Ask all three and remember the answers.
    pub async fn capture(base_url: &str, token: &str) -> Self {
        Self {
            near_zurich: within(base_url, token, ZURICH, 5_000).await,
            near_bern: within(base_url, token, BERN, 5_000).await,
            wide: within(base_url, token, BERN, 400_000).await,
        }
    }

    /// Re-ask all three and assert nothing moved. `when` names the moment.
    pub async fn assert_unchanged(&self, base_url: &str, token: &str, when: &str) {
        let now = Self::capture(base_url, token).await;
        assert_eq!(
            now.near_zurich, self.near_zurich,
            "5 km around Zurich changed {when}"
        );
        assert_eq!(
            now.near_bern, self.near_bern,
            "5 km around Bern changed {when}"
        );
        assert_eq!(now.wide, self.wide, "400 km around Bern changed {when}");
    }
}
