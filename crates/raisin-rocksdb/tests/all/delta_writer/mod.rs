//! Plan Phase 7: one delta writer that skips unchanged index writes; plan
//! Phase 8: the compound reader, writer and replica writes (`compound_*`);
//! plan Phase 7b: the per-node commit lock (`commit_lock_tests`) and the
//! default-on flag with its automatic rebuild (`auto_rebuild_tests`).
//!
//! Every test here runs with `index.skip_unchanged` ON and the branch rebuilt
//! by the `property_index` repair (unless it says otherwise), so the writers
//! actually take the skip path.

mod auto_rebuild_tests;
mod commit_lock_review_tests;
mod commit_lock_tests;
mod compound_env;
mod compound_merge_review_tests;
mod compound_replica_tests;
mod compound_review_tests;
mod compound_tests;
mod env;
mod merge_tests;
mod node_types;
mod ordering_tests;
mod property_tests;
mod repair_tests;
mod replica_tests;
mod review_origin_tests;
mod review_replica_tests;
mod review_state_tests;
