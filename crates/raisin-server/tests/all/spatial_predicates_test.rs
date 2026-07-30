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

//! Topological predicates end to end against a real server.
//!
//! **This module holds NO tests. The coverage described below was delivered in
//! [`crate::st_conformance`] instead — see `st_conformance/predicates/` (the
//! 490-combination predicate x type-pair sweep and the algebraic-identity
//! checks) and `st_conformance/stored/` (the same library re-run over geometry
//! written through SQL).** Run it with:
//!
//! ```text
//! cargo test -p raisin-server --test all st_conformance -- --ignored --nocapture
//! ```
//!
//! This file is kept only for the design rationale below, which records why the
//! coverage had to be shaped per type PAIR rather than per function. Do not read
//! a passing `spatial_predicates_test` run as evidence of anything: it selects
//! zero tests.
//!
//! The ten DE-9IM predicates (`ST_INTERSECTS`, `ST_CONTAINS`, `ST_WITHIN`,
//! `ST_TOUCHES`, `ST_CROSSES`, `ST_OVERLAPS`, `ST_DISJOINT`, `ST_EQUALS`,
//! `ST_COVERS`, `ST_COVEREDBY`) plus `ST_RELATE` must be exercised against a
//! real server over real stored data — unit tests are explicitly not accepted as
//! proof here.
//!
//! Cover every geometry **type pair**, not merely every function. The old
//! implementation hand-rolled a match arm per type pair, so each predicate
//! supported a different, incomplete set: `ST_CROSSES` and `ST_TOUCHES`
//! hardcoded `false` for Point arguments and `ST_OVERLAPS` had a silent
//! catch-all `false`. A per-function test would have passed throughout.
//!
//! Include `Multi*` and `GeometryCollection` on both sides: after the conversion
//! layer landed, an `_ => Err(unsupported)` arm anywhere is a bug.
