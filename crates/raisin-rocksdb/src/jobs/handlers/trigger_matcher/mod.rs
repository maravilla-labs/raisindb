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

//! Trigger matcher implementation
//!
//! This module provides functions to create trigger matcher callbacks
//! that query raisin:Function nodes to find matching triggers for events.

mod filters;
mod inline_triggers;
mod matcher;
mod standalone_triggers;

#[cfg(test)]
mod tests;

pub use self::matcher::create_trigger_matcher;
