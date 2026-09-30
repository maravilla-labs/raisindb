//! Stamps the component with a fingerprint of its own sources.
//!
//! The artifact is committed, and CI has no wasm toolchain to rebuild it, so an
//! edit to `src/` that is not followed by `make ai-tools-rag` would ship the OLD
//! behaviour under the new source. `raisin-functions`' artifact test computes
//! the same fingerprint over the same files and compares it with what the
//! committed component reports from its `default` handler: a stale artifact is
//! a red test, not a surprise in production.
//!
//! FNV-1a 64 over `src/*.rs` in name order, file name then contents, with CR
//! bytes skipped so a CRLF checkout fingerprints the same. Deliberately tiny and
//! dependency-free: the host side reimplements it.

use std::fs;
use std::path::Path;

fn main() {
    let dir = Path::new("src");
    let mut names: Vec<String> = fs::read_dir(dir)
        .expect("src/ is readable")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".rs"))
        .collect();
    names.sort();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for name in &names {
        println!("cargo:rerun-if-changed=src/{name}");
        let bytes = fs::read(dir.join(name)).expect("source is readable");
        for b in name.as_bytes().iter().chain(bytes.iter()) {
            if *b == b'\r' {
                continue;
            }
            hash ^= u64::from(*b);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rustc-env=AI_TOOLS_RAG_SOURCE_HASH={hash:016x}");
}
