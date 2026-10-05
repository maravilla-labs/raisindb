use super::*;
use std::sync::Arc;

fn db() -> (tempfile::TempDir, DB) {
    let dir = tempfile::TempDir::new().unwrap();
    let mut opts = rocksdb::Options::default();
    opts.create_if_missing(true);
    let db = DB::open(&opts, dir.path()).unwrap();
    (dir, db)
}

#[test]
fn an_inserter_blocks_a_collapse_and_bumps_the_epoch() {
    let (_dir, db) = db();
    let first = try_collapse(&db, "t", "r", "main", "cf").expect("free");
    let start = first.epoch();
    drop(first);
    let inserter = enter_inserter(&db, "t", "r", "main", "cf");
    assert!(try_collapse(&db, "t", "r", "main", "cf").is_none());
    // Another branch or CF is independent.
    assert!(try_collapse(&db, "t", "r", "other", "cf").is_some());
    drop(inserter);
    let again = try_collapse(&db, "t", "r", "main", "cf").expect("released");
    assert_eq!(again.epoch(), start + 1);
}

#[test]
fn an_inserter_waits_out_a_collapse_slice() {
    let (_dir, db) = db();
    let db = Arc::new(db);
    let slice = try_collapse(&db, "t", "r", "main", "cf").expect("free");
    let waiter = {
        let db = db.clone();
        std::thread::spawn(move || {
            let _g = enter_inserter(&db, "t", "r", "main", "cf");
        })
    };
    std::thread::sleep(Duration::from_millis(50));
    assert!(
        !waiter.is_finished(),
        "the inserter must wait for the slice"
    );
    drop(slice);
    waiter.join().unwrap();
}

#[test]
fn a_waiting_inserter_is_not_starved_by_the_next_slice() {
    let (_dir, db) = db();
    let db = Arc::new(db);
    let slice = try_collapse(&db, "t", "r", "main", "cf").expect("free");
    let (tx, rx) = std::sync::mpsc::channel();
    let waiter = {
        let db = db.clone();
        std::thread::spawn(move || {
            let g = enter_inserter(&db, "t", "r", "main", "cf");
            tx.send(()).unwrap();
            std::thread::sleep(Duration::from_millis(50));
            drop(g);
        })
    };
    std::thread::sleep(Duration::from_millis(50));
    // The slice ends: whether or not the inserter has woken yet, the next
    // slice must not start ahead of it.
    drop(slice);
    assert!(try_collapse(&db, "t", "r", "main", "cf").is_none());
    rx.recv().unwrap();
    waiter.join().unwrap();
    assert!(try_collapse(&db, "t", "r", "main", "cf").is_some());
}

#[test]
fn retention_gc_and_collapse_exclude_each_other() {
    let (_dir, db) = db();
    let db = Arc::new(db);
    let before = try_collapse(&db, "t", "r", "main", "cf")
        .expect("free")
        .epoch();

    // A running retention GC refuses every slice of the database.
    let pruner = enter_pruner(&db);
    assert!(try_collapse(&db, "t", "r", "main", "cf").is_none());
    assert!(try_collapse(&db, "t", "r", "other", "cf2").is_none());
    drop(pruner);
    let slice = try_collapse(&db, "t", "r", "main", "cf").expect("released");
    assert_ne!(slice.epoch(), before, "a pruner run must bump the epoch");

    // A retention GC waits out a slice in progress.
    let waiter = {
        let db = db.clone();
        std::thread::spawn(move || {
            let _g = enter_pruner(&db);
        })
    };
    std::thread::sleep(Duration::from_millis(50));
    assert!(!waiter.is_finished(), "the pruner must wait for the slice");
    drop(slice);
    waiter.join().unwrap();

    // Another database is independent.
    let (_dir2, other) = self::db();
    let _p = enter_pruner(&db);
    assert!(try_collapse(&other, "t", "r", "main", "cf").is_some());
}
