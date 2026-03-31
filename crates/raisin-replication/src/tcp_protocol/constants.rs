//! Protocol constants for TCP replication

/// Current protocol version
pub const PROTOCOL_VERSION: u8 = 1;

/// Maximum message size (10MB)
pub const MAX_MESSAGE_SIZE: usize = 10 * 1024 * 1024;

/// Default batch size for operation transfer
pub const DEFAULT_BATCH_SIZE: usize = 1000;

/// Default maximum parallel file transfers
pub const DEFAULT_MAX_PARALLEL_FILES: u8 = 4;

/// Default batch size for serde
pub(crate) fn default_batch_size() -> usize {
    DEFAULT_BATCH_SIZE
}

/// Default max parallel files for serde
pub(crate) fn default_max_parallel_files() -> u8 {
    DEFAULT_MAX_PARALLEL_FILES
}
