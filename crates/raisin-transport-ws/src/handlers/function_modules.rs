// SPDX-License-Identifier: BSL-1.1

//! Importable module files for a function this transport executes inline.
//!
//! `LoadedFunction::files` is what the QuickJS resolver consults, and the loader
//! is rebuilt per execution for tenant isolation — so an empty map rejects
//! **every** `import`, and a function split across sibling files fails at
//! declare time with `Error resolving module '…' from 'entry'`.
//!
//! Every path that builds a `LoadedFunction` by hand owes this call. The ones
//! that go through the job executor get it for free from
//! `raisin_functions::execution::executor::load_module_set`, which is what made
//! the omission so hard to see: the same function works from a trigger, a flow
//! or a sync run, and fails only when invoked over this transport.
//!
//! One helper rather than a copy per call site, because two implementations of
//! "which files can this function import" is exactly the mirrored-code drift
//! this codebase keeps paying for.

use std::collections::HashMap;

use raisin_binary::BinaryStorage;
use raisin_storage::Storage;

/// Load a function's sibling modules plus any `../dir/` externals it imports.
///
/// Both loads are best-effort: a function that imports nothing must not fail
/// because a directory scan did, and a genuinely missing module still surfaces
/// as a resolver error at declare time — where the message names the specifier.
///
/// `workspace` is the caller's, not a constant: a SQL query may name another
/// workspace, and scanning the wrong one yields an empty map that is
/// indistinguishable at the resolver from "this function has no modules".
#[allow(clippy::too_many_arguments)]
pub(crate) async fn load_function_modules<S, B>(
    storage: &S,
    bin: &B,
    tenant_id: &str,
    repo: &str,
    branch: &str,
    workspace: &str,
    function_path: &str,
    entry_file_name: &str,
    entry_code: &str,
) -> HashMap<String, String>
where
    S: Storage,
    B: BinaryStorage,
{
    use raisin_functions::execution::code_loader;

    let mut files = code_loader::load_sibling_files(
        storage,
        bin,
        tenant_id,
        repo,
        branch,
        workspace,
        function_path,
        entry_file_name,
    )
    .await
    .unwrap_or_default();

    let external = code_loader::load_external_modules(
        storage,
        bin,
        tenant_id,
        repo,
        branch,
        workspace,
        function_path,
        entry_code,
        &files,
    )
    .await
    .unwrap_or_default();
    files.extend(external);

    files
}
