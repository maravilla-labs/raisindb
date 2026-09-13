//! usearch must never close a file descriptor it did not open.
//!
//! usearch 2.26.1's `memory_mapped_file_t` defaulted its descriptor to 0, so
//! `close()` after a failed `view` — and the destructor's second `close()` after
//! a successful one — called `::close(0)`. In the server, fd 0 is whatever the
//! kernel last handed out at that number: on 2026-09-13 it was the RocksDB WAL,
//! and every write for every tenant failed with "Bad file descriptor" until a
//! restart. The workspace patches usearch (`third_party/usearch`); these tests
//! hold that patch in place.
//!
//! Each test parks a real file on fd 0 and asserts it is still open afterwards.
//! This file is its own test binary, and the tests share a lock, because fd 0 is
//! process-wide.

#![cfg(unix)]

use std::os::fd::AsRawFd;
use std::sync::Mutex;

use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

static FD0: Mutex<()> = Mutex::new(());

/// Put a temp file on fd 0 and return it (keep it alive for the test).
fn park_file_on_fd0() -> std::fs::File {
    let file = tempfile::tempfile().expect("tempfile");
    let rc = unsafe { libc::dup2(file.as_raw_fd(), 0) };
    assert_eq!(rc, 0, "dup2 onto fd 0");
    file
}

fn fd0_is_open() -> bool {
    unsafe { libc::fcntl(0, libc::F_GETFD) != -1 }
}

fn index() -> Index {
    let options = IndexOptions {
        dimensions: 4,
        metric: MetricKind::Cos,
        quantization: ScalarKind::F32,
        ..Default::default()
    };
    Index::new(&options).expect("index")
}

#[test]
fn a_failed_view_leaves_fd_zero_open() {
    let _guard = FD0.lock().unwrap_or_else(|e| e.into_inner());
    let _parked = park_file_on_fd0();

    let index = index();
    assert!(index.view("/nonexistent/raisin-hnsw-fd0.usearch").is_err());
    drop(index);

    assert!(fd0_is_open(), "a failed usearch view closed fd 0");
}

#[test]
fn viewing_and_dropping_an_index_leaves_fd_zero_open() {
    let _guard = FD0.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("index.usearch");
    let path = path.to_str().expect("utf-8 path");

    let source = index();
    source.reserve(2).expect("reserve");
    source.add(1, &[0.1f32, 0.2, 0.3, 0.4]).expect("add");
    source.save(path).expect("save");

    let _parked = park_file_on_fd0();
    let viewed = index();
    viewed.view(path).expect("view");
    viewed.reset().expect("reset");
    drop(viewed);

    assert!(fd0_is_open(), "closing a viewed usearch index closed fd 0");
}
