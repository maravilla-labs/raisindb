//! Notes the planner leaves for `EXPLAIN`: why an access path the query
//! matched was NOT used (a compound index that is not built, built from another
//! declaration, or below its history floor). Without them a declared index was
//! passed over silently and the plan simply looked slow.
//!
//! Collected per `EXPLAIN` in a task-local scope; outside one, a note is only
//! logged.

use std::cell::RefCell;

tokio::task_local! {
    static NOTES: RefCell<Vec<String>>;
}

/// Record a note (deduplicated) for the `EXPLAIN` running on this task, if any.
pub(crate) fn note(text: String) {
    let _ = NOTES.try_with(|notes| {
        let mut notes = notes.borrow_mut();
        if !notes.contains(&text) {
            notes.push(text);
        }
    });
}

/// Run `fut` collecting the planner's notes; returns its output and the notes.
pub(crate) async fn collect<F: std::future::Future>(fut: F) -> (F::Output, Vec<String>) {
    NOTES
        .scope(RefCell::new(Vec::new()), async {
            let out = fut.await;
            let notes = NOTES.with(|n| n.borrow().clone());
            (out, notes)
        })
        .await
}
