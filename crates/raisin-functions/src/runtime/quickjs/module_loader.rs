// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! ES6 module resolution and loading for the QuickJS runtime.
//!
//! Provides module resolution (relative/absolute imports) and loading from
//! the function's in-memory file tree.

use rquickjs::{
    loader::{Loader, Resolver},
    Ctx, Module,
};
use std::collections::HashMap;
use std::sync::Arc;

/// Module resolver for function files.
///
/// Resolves import specifiers to file paths within the function's file tree.
#[derive(Clone)]
pub(super) struct FunctionModuleResolver {
    /// Available files (path -> content)
    files: Arc<HashMap<String, String>>,
}

impl FunctionModuleResolver {
    pub(super) fn new(files: Arc<HashMap<String, String>>) -> Self {
        Self { files }
    }

    /// Resolve a relative import path.
    pub(crate) fn resolve_path(&self, base: &str, specifier: &str) -> String {
        if !specifier.starts_with('.') {
            // Absolute import - return as-is
            return specifier.to_string();
        }

        // Get directory of base path
        let base_dir = if let Some(pos) = base.rfind('/') {
            &base[..pos]
        } else {
            ""
        };

        // Handle relative path components
        let mut parts: Vec<&str> = if base_dir.is_empty() {
            Vec::new()
        } else {
            base_dir.split('/').collect()
        };

        for component in specifier.split('/') {
            match component {
                "." | "" => {}
                ".." => {
                    parts.pop();
                }
                name => parts.push(name),
            }
        }

        let mut result = parts.join("/");

        // Add .js extension if missing
        if !result.ends_with(".js") && !result.ends_with(".mjs") {
            result.push_str(".js");
        }

        result
    }
}

impl Resolver for FunctionModuleResolver {
    fn resolve<'js>(
        &mut self,
        _ctx: &Ctx<'js>,
        base: &str,
        name: &str,
    ) -> rquickjs::Result<String> {
        let resolved = self.resolve_path(base, name);

        if self.files.contains_key(&resolved) {
            Ok(resolved)
        } else {
            Err(rquickjs::Error::new_resolving(base, name))
        }
    }
}

/// Module loader for function files.
///
/// Loads module content from the function's file map.
#[derive(Clone)]
pub(super) struct FunctionModuleLoader {
    /// Available files (path -> content)
    files: Arc<HashMap<String, String>>,
}

impl FunctionModuleLoader {
    pub(super) fn new(files: Arc<HashMap<String, String>>) -> Self {
        Self { files }
    }
}

impl Loader for FunctionModuleLoader {
    fn load<'js>(&mut self, ctx: &Ctx<'js>, name: &str) -> rquickjs::Result<Module<'js>> {
        let content = self
            .files
            .get(name)
            .ok_or_else(|| rquickjs::Error::new_loading(name))?;

        Module::declare(ctx.clone(), name, content.as_str())
    }
}

/// Check if code contains ES6 import/export syntax.
pub(super) fn has_es6_modules(code: &str) -> bool {
    // Simple heuristic: check for import/export keywords at start of lines
    code.lines().any(|line| {
        let trimmed = line.trim();
        trimmed.starts_with("import ")
            || trimmed.starts_with("import{")
            || trimmed.starts_with("export ")
            || trimmed.starts_with("export{")
            || trimmed.starts_with("export default")
    })
}
