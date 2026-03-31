// SPDX-License-Identifier: BSL-1.1

//! Utility functions for the Tantivy indexing engine.

use raisin_error::{Error, Result};

pub(crate) fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> Result<()> {
    std::fs::create_dir_all(dst)
        .map_err(|e| Error::storage(format!("Failed to create target directory: {}", e)))?;

    for entry in std::fs::read_dir(src)
        .map_err(|e| Error::storage(format!("Failed to read source directory: {}", e)))?
    {
        let entry =
            entry.map_err(|e| Error::storage(format!("Failed to read directory entry: {}", e)))?;
        let path = entry.path();
        let file_name = entry.file_name();
        let target_path = dst.join(&file_name);

        if path.is_dir() {
            copy_dir_recursive(&path, &target_path)?;
        } else {
            std::fs::copy(&path, &target_path)
                .map_err(|e| Error::storage(format!("Failed to copy file: {}", e)))?;
        }
    }

    Ok(())
}
