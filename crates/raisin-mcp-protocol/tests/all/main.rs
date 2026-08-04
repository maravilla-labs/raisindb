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

//! Integration tests for `raisin-mcp`.
//!
//! ONE test target for the whole crate (see CLAUDE.md): a new file placed
//! directly under `tests/` links its own binary with the full dependency graph.
//! Add a module here instead.

mod mock;

mod client_session;
