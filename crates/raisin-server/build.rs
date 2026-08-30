//! Build script for raisin-server
//!
//! This script automatically builds the admin console (including WASM SQL validator)
//! when building raisin-server. The admin console assets are then embedded in the
//! binary using rust-embed.

use std::env;
use std::path::Path;
use std::process::Command;

/// Spawn a child that will itself run cargo, with this build script's inherited
/// cargo environment scrubbed.
///
/// EVERY nested build must go through here. There is no such thing as a child
/// of this build script that may keep the outer invocation's cargo environment:
/// each group below is a hang or a hard error, not a preference. Only the two
/// `--version` probes, which build nothing, may use a bare `Command::new`.
///
/// * `CARGO_TARGET_DIR` / `CARGO_BUILD_TARGET_DIR` — **deadlock**. Cargo takes an
///   exclusive `flock` on `<target-dir>/<profile>/.cargo-lock` (the host layout,
///   taken even for a `--target wasm32-*` build) and holds it for the whole
///   build. The outer `cargo build` that is running *us* already holds it. If the
///   nested cargo resolves to the same target dir it blocks on that lock forever,
///   waiting for a parent that is waiting for it. It does say
///   "Blocking waiting for file lock on artifact directory" — but onto a build
///   script's stdout, which cargo captures and only ever prints for a build that
///   *finishes*, so nobody sees it. What you observe instead is a build sitting
///   at 0% CPU with no rustc running and never returning. Anyone who exports
///   `CARGO_TARGET_DIR` (a target dir shared across worktrees is the usual
///   reason) could not build this crate at all. Removing the variables puts each
///   nested crate back on its own crate-local `target/`, which is what every
///   environment that does not export them already does.
/// * `CARGO_ENCODED_RUSTFLAGS` / `RUSTFLAGS` / `CARGO_BUILD_RUSTFLAGS` — **hard
///   error**. The workspace `.cargo/config.toml` sets `-C
///   split-debuginfo=unpacked` for the host target; cargo propagates it to us,
///   and a nested wasm32 build inheriting it fails with
///   "`-Csplit-debuginfo=unpacked` is unstable on this platform".
fn nested_build_command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    cmd.env_remove("CARGO_TARGET_DIR")
        .env_remove("CARGO_BUILD_TARGET_DIR")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_BUILD_RUSTFLAGS");
    cmd
}

fn main() {
    let frontend_dir = "../../packages/admin-console";
    let flow_designer_dir = "../../packages/raisin-flow-designer";
    let raisin_editor_dir = "../../packages/raisin-editor";
    let wasm_dir = "../../tooling/packages/raisin-sql-wasm";
    let wasm_pkg_dir = format!("{}/pkg", wasm_dir);
    let raisin_sql_dir = "../raisin-sql";

    // Watch for source changes that should trigger rebuild
    println!("cargo:rerun-if-changed={}/src", frontend_dir);
    println!("cargo:rerun-if-changed={}/package.json", frontend_dir);
    println!("cargo:rerun-if-changed={}/vite.config.ts", frontend_dir);
    println!("cargo:rerun-if-changed={}/index.html", frontend_dir);
    // Watch flow designer sources used by admin console dependency
    println!("cargo:rerun-if-changed={}/src", flow_designer_dir);
    println!("cargo:rerun-if-changed={}/package.json", flow_designer_dir);
    println!("cargo:rerun-if-changed={}/tsconfig.json", flow_designer_dir);
    // Watch raisin-editor sources used by admin console dependency
    println!("cargo:rerun-if-changed={}/src", raisin_editor_dir);
    println!("cargo:rerun-if-changed={}/package.json", raisin_editor_dir);
    println!("cargo:rerun-if-changed={}/tsconfig.json", raisin_editor_dir);
    // Watch WASM package source
    println!("cargo:rerun-if-changed={}/src", wasm_dir);
    println!("cargo:rerun-if-changed={}/Cargo.toml", wasm_dir);
    // Watch raisin-sql source (WASM depends on it)
    println!("cargo:rerun-if-changed={}/src", raisin_sql_dir);
    println!("cargo:rerun-if-changed={}/Cargo.toml", raisin_sql_dir);

    // Ensure the dist folder exists (rust-embed needs it even when empty)
    let dist_dir = Path::new(".admin-console-dist");
    if !dist_dir.exists() {
        std::fs::create_dir_all(dist_dir).ok();
    }

    // Skip build only if explicitly requested
    if env::var("SKIP_ADMIN_BUILD").is_ok() {
        println!("cargo:warning=Skipping admin console build (SKIP_ADMIN_BUILD set)");
        return;
    }

    // Check if npm/pnpm is available
    let pnpm_available = Command::new("pnpm")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    let npm_available = Command::new("npm")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if !pnpm_available && !npm_available {
        println!("cargo:warning=Neither pnpm nor npm found, skipping frontend build");
        println!("cargo:warning=The admin console will not be available");
        return;
    }

    let pkg_cmd = if pnpm_available { "pnpm" } else { "npm" };

    // Step 1: Build WASM module
    // Always rebuild to ensure changes in raisin-sql are picked up
    println!("cargo:warning=Building WASM SQL validator...");

    // Check if wasm-pack is installed
    let wasm_pack_check = Command::new("wasm-pack").arg("--version").output();

    if wasm_pack_check.is_err() || !wasm_pack_check.as_ref().unwrap().status.success() {
        println!("cargo:warning=Installing wasm-pack...");
        let install_status = nested_build_command("cargo")
            .args(["install", "wasm-pack"])
            .status()
            .expect("Failed to install wasm-pack");

        if !install_status.success() {
            println!(
                "cargo:warning=Failed to install wasm-pack, WASM validation will not be available"
            );
        }
    }

    // Build WASM. See `nested_build_command` for why the inherited cargo
    // environment must be scrubbed rather than passed through.
    let wasm_status = nested_build_command("wasm-pack")
        .args(["build", "--target", "web", "--out-dir", "pkg", "--release"])
        .current_dir(wasm_dir)
        .status();

    match wasm_status {
        Ok(status) if status.success() => {
            // Create package.json for the WASM package
            let package_json = r#"{
  "name": "@raisindb/sql-wasm",
  "version": "0.1.0",
  "description": "WASM bindings for RaisinDB SQL parser validation",
  "main": "raisin_sql_wasm.js",
  "types": "raisin_sql_wasm.d.ts",
  "files": [
    "raisin_sql_wasm_bg.wasm",
    "raisin_sql_wasm.js",
    "raisin_sql_wasm.d.ts"
  ],
  "sideEffects": false
}"#;
            let _ = std::fs::write(format!("{}/package.json", wasm_pkg_dir), package_json);
            println!("cargo:warning=WASM module built successfully");

            // Step 1b: Install WASM package in admin-console
            // Path from admin-console to wasm pkg: ../../tooling/packages/raisin-sql-wasm/pkg
            println!("cargo:warning=Installing WASM package in admin-console...");
            let install_wasm_status = nested_build_command(pkg_cmd)
                .args(["install", "../../tooling/packages/raisin-sql-wasm/pkg"])
                .current_dir(frontend_dir)
                .status();

            match install_wasm_status {
                Ok(status) if status.success() => {
                    println!("cargo:warning=WASM package installed in admin-console");
                }
                _ => {
                    println!("cargo:warning=Failed to install WASM package in admin-console");
                }
            }
        }
        _ => {
            println!(
                "cargo:warning=Failed to build WASM module, SQL validation will not be available"
            );
        }
    }

    // Step 2: Check if node_modules exists
    if !Path::new(frontend_dir).join("node_modules").exists() {
        println!("cargo:warning=Installing dependencies for admin-console...");
        let status = nested_build_command(pkg_cmd)
            .args(["install"])
            .current_dir(frontend_dir)
            .status()
            .expect("Failed to run npm install");

        if !status.success() {
            println!("cargo:warning=npm install failed");
            return;
        }
    }

    // Step 3: Build the frontend
    println!("cargo:warning=Building admin-console frontend...");
    // The frontend prebuild shells out to `raisin-rel-wasm/build.sh`, i.e. one
    // more nested wasm-pack: same scrub, same reasons.
    let status = nested_build_command(pkg_cmd)
        .args(["run", "build"])
        .current_dir(frontend_dir)
        .status()
        .expect("Failed to build frontend");

    if !status.success() {
        panic!("Frontend build failed");
    }

    println!("cargo:warning=Admin console built successfully");
}
