//! Process-wide registry of in-memory caches derived from stored data.
//!
//! Several subsystems cache things they derived from storage — the SQL
//! workspace catalog, function module sets, schema statistics. Each keeps itself
//! correct by listening for the events its source data emits.
//!
//! That contract breaks for **bulk operations that bypass the write path
//! entirely**. Replication checkpoint ingestion is the case that motivated this:
//! it copies whole column families straight into the live database, deliberately
//! emitting no events (re-running the init handlers would try to re-initialize
//! data the checkpoint already contains). Every event-driven cache in the
//! process is therefore silently wrong afterwards — a catalog that has never
//! heard of the workspaces just ingested answers "unknown table" for them.
//!
//! Such an operation calls [`invalidate_all_derived_caches`]. Caches opt in with
//! [`register_invalidator`], normally from inside their lazy initializer, so a
//! cache that was never populated registers nothing and needs nothing: it has no
//! stale state to drop and will read the post-ingest world on first use.
//!
//! This is deliberately a blunt instrument. It is for the rare bulk path, not
//! for ordinary writes — those must keep using their own targeted invalidation,
//! which is both cheaper and more precise.

use std::sync::{Mutex, OnceLock};

type Invalidator = Box<dyn Fn() + Send + Sync>;

static INVALIDATORS: OnceLock<Mutex<Vec<Invalidator>>> = OnceLock::new();

fn invalidators() -> &'static Mutex<Vec<Invalidator>> {
    INVALIDATORS.get_or_init(|| Mutex::new(Vec::new()))
}

/// Register a callback that drops every entry of one derived cache.
///
/// Call this from the cache's lazy initializer so registration and population
/// cannot get out of order.
pub fn register_invalidator<F>(invalidate: F)
where
    F: Fn() + Send + Sync + 'static,
{
    if let Ok(mut guard) = invalidators().lock() {
        guard.push(Box::new(invalidate));
    }
}

/// Drop every registered derived cache.
///
/// Call after any bulk operation that writes stored data without going through
/// the normal, event-emitting write path.
pub fn invalidate_all_derived_caches() {
    let Ok(guard) = invalidators().lock() else {
        tracing::error!("Derived cache registry poisoned; caches may serve stale data");
        return;
    };

    for invalidate in guard.iter() {
        invalidate();
    }

    tracing::info!(
        count = guard.len(),
        "Invalidated all derived caches after a bulk storage operation"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn registered_invalidators_all_run() {
        let hits = Arc::new(AtomicUsize::new(0));

        for _ in 0..3 {
            let hits = hits.clone();
            register_invalidator(move || {
                hits.fetch_add(1, Ordering::SeqCst);
            });
        }

        invalidate_all_derived_caches();

        // The registry is process-wide, so other tests in this binary may have
        // registered too; assert on ours rather than on the total.
        assert!(
            hits.load(Ordering::SeqCst) >= 3,
            "every registered invalidator must run"
        );
    }
}
