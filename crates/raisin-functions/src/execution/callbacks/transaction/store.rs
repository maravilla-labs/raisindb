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

//! Transaction store for managing active QueryEngine instances.

use std::sync::Arc;

use dashmap::DashMap;
use raisin_sql_execution::QueryEngine;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::Storage;
use tokio::sync::Mutex;

/// Store for active transactions.
///
/// Transactions are stored by a unique ID. Each transaction holds a QueryEngine
/// instance with an active SQL transaction (BEGIN has been executed).
pub struct TransactionStore<S>
where
    S: Storage + TransactionalStorage + 'static,
{
    /// Map of transaction ID to QueryEngine with active transaction
    transactions: DashMap<String, Arc<Mutex<QueryEngine<S>>>>,
}

impl<S> TransactionStore<S>
where
    S: Storage + TransactionalStorage + 'static,
{
    /// Create a new empty transaction store.
    pub fn new() -> Self {
        Self {
            transactions: DashMap::new(),
        }
    }

    /// Insert a transaction engine into the store.
    pub fn insert(&self, id: String, engine: Arc<Mutex<QueryEngine<S>>>) {
        self.transactions.insert(id, engine);
    }

    /// Remove and return a transaction engine from the store.
    pub fn remove(&self, id: &str) -> Option<Arc<Mutex<QueryEngine<S>>>> {
        self.transactions.remove(id).map(|(_, v)| v)
    }

    /// Get a reference to a transaction engine.
    pub fn get(&self, id: &str) -> Option<Arc<Mutex<QueryEngine<S>>>> {
        self.transactions.get(id).map(|r| r.clone())
    }
}

impl<S> Default for TransactionStore<S>
where
    S: Storage + TransactionalStorage + 'static,
{
    fn default() -> Self {
        Self::new()
    }
}
