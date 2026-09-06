// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Regenerate the WebAssembly guest-SDK bindings from the bindings registry.
//!
//! ```text
//! cargo run -p raisin-functions --bin gen-bindings            # write
//! cargo run -p raisin-functions --bin gen-bindings -- --check # verify only
//! ```
//!
//! `--check` exits non-zero when a committed file differs from what the
//! registry renders — the same condition the freshness tests assert.

use raisin_functions::runtime::bindings::gen;

fn main() -> std::io::Result<()> {
    let check = std::env::args().skip(1).any(|a| a == "--check");
    let root = gen::repo_root();

    if check {
        let stale = gen::stale_files(&root);
        if stale.is_empty() {
            println!(
                "gen-bindings: up to date ({} files)",
                gen::render_all().len()
            );
            return Ok(());
        }
        eprintln!("gen-bindings: {} file(s) out of date:", stale.len());
        for path in stale {
            eprintln!("  {}", path);
        }
        eprintln!("{}", gen::REGENERATE_HINT);
        std::process::exit(1);
    }

    let changed = gen::write_all(&root)?;
    if changed.is_empty() {
        println!("gen-bindings: already up to date");
    } else {
        println!("gen-bindings: wrote {} file(s):", changed.len());
        for path in changed {
            println!("  {}", path);
        }
    }
    Ok(())
}
