//! Run-collapse GC (plan Phase 9) and the retention GC behaviours it relies
//! on. Kept beside `history_gc_test` (already over 400 lines) rather than in
//! it.

mod collapse_tests;
mod env;
mod gc_copy_tests;
mod gc_tests;
mod prereq_tests;
mod race_tests;
mod watermark_tests;
