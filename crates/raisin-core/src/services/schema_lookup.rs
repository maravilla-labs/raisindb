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

//! The one thing schema inheritance resolution actually needs: "fetch this
//! NodeType / ElementType by name".
//!
//! # Why this exists
//!
//! [`NodeTypeResolver`](super::node_type_resolver::NodeTypeResolver) and
//! [`ElementTypeResolver`](super::element_type_resolver::ElementTypeResolver)
//! were generic over the whole [`Storage`] trait, but their resolution bodies
//! touch exactly one repository each. That was fine until a caller BELOW the
//! storage facade needed inheritance-aware resolution: the RocksDB node
//! repository has an `Arc<NodeTypeRepositoryImpl>` and can never have an
//! `Arc<RocksDBStorage>` — the storage owns the repository, so holding one back
//! would be a reference cycle that never drops the database handle.
//!
//! The alternative was a second, repository-local inheritance walk. That is the
//! failure mode this codebase names as its number one recurring bug class, and
//! for `encrypted` in particular a walker that misses an INHERITED secret
//! declaration writes a plaintext password to disk. So the resolvers keep ONE
//! body and take their input through these traits instead.
//!
//! # Why not `dyn NodeTypeRepository`
//!
//! [`raisin_storage::NodeTypeRepository`] returns `impl Future` (RPITIT), which
//! is not object-safe — `dyn NodeTypeRepository` does not compile. These traits
//! are `#[async_trait]` (boxed futures) precisely so a resolver can hold one
//! behind an `Arc<dyn …>` without knowing the concrete backend.

use async_trait::async_trait;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::types::element::element_type::ElementType;
use raisin_models::nodes::types::NodeType;

/// Fetch a NodeType by name within one branch scope.
#[async_trait]
pub trait NodeTypeLookup: Send + Sync {
    async fn get_node_type(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        name: &str,
        max_revision: Option<HLC>,
    ) -> Result<Option<NodeType>>;
}

/// Fetch an ElementType by name within one branch scope.
#[async_trait]
pub trait ElementTypeLookup: Send + Sync {
    async fn get_element_type(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        name: &str,
    ) -> Result<Option<ElementType>>;
}

// NOTE: there is deliberately no blanket `impl<S: Storage> NodeTypeLookup for
// Arc<S>` adapter here. Coercing such an adapter into `Arc<dyn NodeTypeLookup>`
// would require `S: 'static`, and that bound propagates from the resolvers into
// `NodeValidator`, `NodeService` and every transport that names them — dozens
// of files, to serve one call site. The resolvers instead hold a two-variant
// source (storage handle OR lookup), so only the FETCH differs and the
// inheritance walk itself stays single.
