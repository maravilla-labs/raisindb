// SPDX-License-Identifier: BSL-1.1

//! Every SQL statement context carries the engine's `sql.batched_fetch`.
//!
//! The scalar path (`SELECT RESOLVE('{...}'::jsonb, 2)`, no FROM) once built
//! its context by hand and kept the process default, so an engine with the
//! rollback switch off still resolved through the batched reader. Every
//! construction site now goes through `new_statement_context`.

use super::QueryEngine;
use raisin_storage_memory::InMemoryStorage;
use std::sync::Arc;

fn engine() -> QueryEngine<InMemoryStorage> {
    QueryEngine::new(Arc::new(InMemoryStorage::default()), "t", "repo", "main")
}

#[test]
fn scalar_query_context_honours_batched_fetch_off() {
    for batched in [false, true] {
        let ctx = engine()
            .with_batched_fetch(batched)
            .new_statement_context("main".into(), "default".into());
        assert_eq!(ctx.batched_fetch, batched);
    }
}
