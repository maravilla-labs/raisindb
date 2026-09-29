//! Compiles `resume_shim.cc` against the RocksDB headers librocksdb-sys ships.
//! See the header of Cargo.toml for why this vendor copy exists.

fn main() {
    // librocksdb-sys declares `links = "rocksdb"` and prints
    // `cargo:cargo_manifest_dir`, which reaches this build script as
    // DEP_ROCKSDB_CARGO_MANIFEST_DIR.
    let sys_dir = std::env::var("DEP_ROCKSDB_CARGO_MANIFEST_DIR")
        .expect("librocksdb-sys must export its manifest dir");
    let std = std::env::var("ROCKSDB_CXX_STD").unwrap_or_else(|_| "c++17".to_string());
    let std = if std.starts_with("-std=") {
        std
    } else {
        format!("-std={std}")
    };

    let mut build = cc::Build::new();
    build
        .cpp(true)
        .file("resume_shim.cc")
        .include(format!("{sys_dir}/rocksdb/include"))
        .define("NDEBUG", Some("1"));
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        build.flag("-std:c++17");
    } else {
        build.flag(&std);
    }
    build.compile("raisin_rocksdb_resume");

    println!("cargo:rerun-if-changed=resume_shim.cc");
    println!("cargo:rerun-if-env-changed=ROCKSDB_CXX_STD");
}
