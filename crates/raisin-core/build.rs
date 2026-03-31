fn main() {
    // Trigger recompilation when global_nodetypes directory changes
    println!("cargo:rerun-if-changed=global_nodetypes");

    // Also watch individual YAML files for more granular detection
    if let Ok(entries) = std::fs::read_dir("global_nodetypes") {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "yaml") {
                println!("cargo:rerun-if-changed={}", path.display());
            }
        }
    }

    // Watch global_workspaces directory too (same pattern)
    println!("cargo:rerun-if-changed=global_workspaces");
    if let Ok(entries) = std::fs::read_dir("global_workspaces") {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "yaml") {
                println!("cargo:rerun-if-changed={}", path.display());
            }
        }
    }
}
