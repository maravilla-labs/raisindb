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
//!
//! **Database scope.** A bulk copy lands in ONE database, and a cache keyed by
//! database (several databases can share a process: tests, an embedded host)
//! should drop only that database's entries — dropping another database's is
//! not merely wasted work when a cold entry has a cost of its own (the compound
//! definitions cache fails a workspace's indexes closed on a cold read). Such a
//! cache registers with [`register_database_invalidator`]; the bulk path calls
//! [`invalidate_derived_caches_for_database`] with the database's path as
//! `rocksdb::DB::path` reports it (lossy UTF-8). Caches registered with
//! [`register_invalidator`] drop everything either way.

use std::sync::{Mutex, OnceLock};

/// `None` = every database; `Some(path)` = only the database at `path`.
type Invalidator = Box<dyn Fn(Option<&str>) + Send + Sync>;

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
    register_database_invalidator(move |_| invalidate());
}

/// Register a cache keyed by database: the callback receives `None` (drop
/// every database's entries) or `Some(path)` (drop only that database's).
pub fn register_database_invalidator<F>(invalidate: F)
where
    F: Fn(Option<&str>) + Send + Sync + 'static,
{
    if let Ok(mut guard) = invalidators().lock() {
        guard.push(Box::new(invalidate));
    }
}

/// Drop every registered derived cache, in every database.
///
/// Call after any bulk operation that writes stored data without going through
/// the normal, event-emitting write path, when it cannot name the database.
pub fn invalidate_all_derived_caches() {
    run_invalidators(None);
}

/// Drop every registered derived cache after a bulk operation into the ONE
/// database at `database_path` (`rocksdb::DB::path`, lossy UTF-8): caches
/// keyed by database drop only its entries, every other cache drops all.
pub fn invalidate_derived_caches_for_database(database_path: &str) {
    run_invalidators(Some(database_path));
}

fn run_invalidators(database: Option<&str>) {
    let Ok(guard) = invalidators().lock() else {
        tracing::error!("Derived cache registry poisoned; caches may serve stale data");
        return;
    };

    for invalidate in guard.iter() {
        invalidate(database);
    }

    tracing::info!(
        count = guard.len(),
        database = database.unwrap_or("*"),
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

    #[test]
    fn a_database_scoped_bulk_path_names_its_database() {
        let seen = Arc::new(Mutex::new(Vec::<Option<String>>::new()));
        let unscoped = Arc::new(AtomicUsize::new(0));
        {
            let seen = seen.clone();
            register_database_invalidator(move |db| {
                seen.lock().unwrap().push(db.map(str::to_string));
            });
        }
        {
            let unscoped = unscoped.clone();
            register_invalidator(move || {
                unscoped.fetch_add(1, Ordering::SeqCst);
            });
        }

        invalidate_derived_caches_for_database("/data/a");

        assert!(seen.lock().unwrap().contains(&Some("/data/a".to_string())));
        assert!(
            unscoped.load(Ordering::SeqCst) >= 1,
            "a cache that cannot scope still drops everything"
        );
    }
}
