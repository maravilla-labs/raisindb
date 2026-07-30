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

//! Constructors, output, accessors, and the 3-D family.

use super::harness::Ctx;

pub async fn run(ctx: &mut Ctx) {
    println!("\n=== constructors & output ===");
    constructors(ctx).await;
    println!("\n=== accessors ===");
    accessors(ctx).await;
    println!("\n=== line access ===");
    line_access(ctx).await;
    println!("\n=== three dimensions ===");
    three_d(ctx).await;
}

mod build;
mod read;

use build::constructors;
use read::{accessors, line_access, three_d};
