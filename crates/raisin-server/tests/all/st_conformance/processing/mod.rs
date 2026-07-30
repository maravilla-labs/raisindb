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

//! Processing (buffer, centroid, envelope, hull, simplify, reverse, boundary),
//! set operations, and validation.

use super::harness::Ctx;

pub async fn run(ctx: &mut Ctx) {
    println!("\n=== buffer ===");
    buffer(ctx).await;
    println!("\n=== processing ===");
    shapes(ctx).await;
    println!("\n=== set operations ===");
    setops(ctx).await;
    println!("\n=== validation ===");
    validation(ctx).await;
}

mod buffer;
mod setops;
mod shapes;
mod validation;

use buffer::buffer;
use setops::setops;
use shapes::shapes;
use validation::validation;
