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

pub mod archetype;
pub mod element;
pub mod initial_structure;
// Split node_type into submodules for readability
pub mod node_type {
    pub mod model;
    pub mod validation;
    pub use model::*;
}
pub mod utils;

pub use archetype::*;
pub use node_type::*;
pub use utils::*;
