//! The `workspaces => ...` argument: its grammar, and how it resolves to a set.
//!
//! One parser and one resolver, shared by `HYBRID_SEARCH`, `FULLTEXT_SEARCH` and
//! `KNN`. The universe a search covers is the single most consequential thing
//! about it and it used to be invisible: the two-argument form searched every
//! workspace in the repository, which is not written anywhere in the query, was
//! not in the documentation, and could only be discovered by noticing build
//! artifacts in the results of a question about documents.

use std::collections::BTreeSet;

use raisin_core::services::rls_filter::{readable_workspaces, ReadableWorkspaces};
use raisin_models::auth::AuthContext;
use raisin_storage::Storage;

use crate::physical_plan::executor::ExecutionError;

/// The spelling the caller wrote, parsed but not yet resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceScopeSpec {
    /// `'library'` or `'library, handbook, policies'`. Naming a workspace is an
    /// ASSERTION: a name that does not resolve is an error.
    Exact(Vec<String>),
    /// `'content-*'`. Matching a pattern is a QUERY: names that do not resolve
    /// are simply not matched, and that is not an error.
    Glob(String),
    /// `'ALL READABLE'`. Every workspace this caller may read.
    AllReadable,
}

impl WorkspaceScopeSpec {
    /// True when a name that fails to resolve must be reported rather than
    /// silently dropped.
    fn names_are_assertions(&self) -> bool {
        matches!(self, WorkspaceScopeSpec::Exact(_))
    }
}

/// Parse the `workspaces` argument value.
///
/// ```text
/// exact := <name>                        'library'
/// set   := <name> ("," <name>)+          'library, handbook, policies'
/// glob  := token containing '*' or '?'   'content-*'
/// all   := 'ALL READABLE'                case-insensitive, exactly two words
/// error := '' | '*' | 'ALL' | anything else
/// ```
///
/// `'*'` is rejected on purpose. It reads as "unscoped" and the eye slides over
/// it; `'ALL READABLE'` is two uppercase words that appear in no other context,
/// so an operator asked "which of our queries go repo-wide?" can answer with one
/// grep. **Do not add a shorter alias later** -- that is how this becomes `'*'`
/// again.
pub fn parse_workspace_scope(raw: &str) -> Result<WorkspaceScopeSpec, ExecutionError> {
    let trimmed = raw.trim();

    if trimmed.is_empty() {
        return Err(ExecutionError::Validation(
            "workspaces => '' is not a scope. Name a workspace, a comma-separated \
             list, a glob such as 'content-*', or 'ALL READABLE' for every \
             workspace you may read."
                .to_string(),
        ));
    }

    // 'ALL READABLE', case-insensitive, exactly two words.
    let words: Vec<&str> = trimmed.split_whitespace().collect();
    if words.len() == 2
        && words[0].eq_ignore_ascii_case("ALL")
        && words[1].eq_ignore_ascii_case("READABLE")
    {
        return Ok(WorkspaceScopeSpec::AllReadable);
    }

    if trimmed == "*" || trimmed.eq_ignore_ascii_case("ALL") {
        return Err(ExecutionError::Validation(format!(
            "workspaces => '{trimmed}' is not a scope. Use 'ALL READABLE' for every \
             workspace you may read, or a glob such as 'content-*'."
        )));
    }

    if trimmed.contains('*') || trimmed.contains('?') {
        // A glob is one token. Reject `'a*, b*'` rather than guessing which
        // half was meant.
        if trimmed.contains(',') {
            return Err(ExecutionError::Validation(format!(
                "workspaces => '{trimmed}' mixes a glob with a list. Use one glob \
                 ('content-*') or a comma-separated list of exact names."
            )));
        }
        // Compile once here so a bad pattern is a parse error rather than a
        // silent zero-match at resolution time.
        glob::Pattern::new(trimmed).map_err(|e| {
            ExecutionError::Validation(format!(
                "workspaces => '{trimmed}' is not a valid glob pattern: {e}"
            ))
        })?;
        return Ok(WorkspaceScopeSpec::Glob(trimmed.to_string()));
    }

    let mut names = Vec::new();
    for part in trimmed.split(',') {
        let name = part.trim();
        if name.is_empty() {
            return Err(ExecutionError::Validation(format!(
                "workspaces => '{trimmed}' has an empty name in its list."
            )));
        }
        names.push(name.to_string());
    }
    Ok(WorkspaceScopeSpec::Exact(names))
}

/// The resolved universe a search runs over.
///
/// `Empty` is NOT `All`. Modelling this as `Option<Vec<String>>` and collapsing
/// an empty vector to `None` -- "no filter" -- turns "may read nothing" into
/// "search everything". That is a complete read-path bypass one keystroke away,
/// which is why the empty case has its own name and its own early return.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceSet {
    /// Push NO workspace filter into either leg. Reached only by a caller RLS
    /// cannot filter (no identity, `is_system`, `is_system_admin`) asking for
    /// `'ALL READABLE'`.
    All,
    /// Push exactly these. Never empty -- an empty resolution is [`Self::Empty`].
    Only(Vec<String>),
    /// Zero rows, without touching either index.
    Empty,
}

impl WorkspaceSet {
    /// The set to push into an index leg, or `None` for "no filter".
    ///
    /// `Empty` also answers `None` because it must never reach a leg at all;
    /// callers check for it first. The `debug_assert` is the tripwire.
    pub fn as_filter(&self) -> Option<&[String]> {
        match self {
            WorkspaceSet::All => None,
            WorkspaceSet::Only(names) => Some(names.as_slice()),
            WorkspaceSet::Empty => {
                debug_assert!(false, "WorkspaceSet::Empty must not reach an index leg");
                None
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        matches!(self, WorkspaceSet::Empty)
    }

    /// How the resolved corpus renders in EXPLAIN and in the operator log.
    pub fn describe(&self) -> String {
        match self {
            WorkspaceSet::All => "ALL (unfiltered: system caller)".to_string(),
            WorkspaceSet::Only(names) => format!("[{}]", names.join(", ")),
            WorkspaceSet::Empty => "[] (nothing readable)".to_string(),
        }
    }
}

/// Counts an operator needs to tell "nothing matched" from "94 matches were
/// unreadable". Never reaches the caller; see `emit`.
#[derive(Debug, Clone, Copy, Default)]
pub struct ScopeStats {
    /// Workspaces in the repo catalog.
    pub catalog: usize,
    /// Of those, how many this caller may read.
    pub readable: usize,
}

/// Expand `spec` against the repo catalog and intersect it with what the caller
/// may read.
///
/// The two error/silence rules are deliberately asymmetric:
///
/// * **exact / set** -- a name lost because it does not exist, OR because the
///   caller may not read it, is an ERROR, and the two produce BYTE-IDENTICAL
///   text. Distinguishing them would make the function an existence oracle:
///   hold everything else constant, vary the name, read the error. The reason
///   goes to a `debug` log instead. Naming a workspace is an assertion, and a
///   typo must not silently shrink a RAG corpus forever.
/// * **glob / ALL READABLE** -- names lost at either step are dropped in
///   silence. Matching a pattern is a query, and a query matching fewer things
///   is not an error.
pub async fn resolve_scope<S: Storage + ?Sized>(
    spec: &WorkspaceScopeSpec,
    storage: &S,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    auth: Option<&AuthContext>,
) -> Result<(WorkspaceSet, ScopeStats), ExecutionError> {
    let catalog =
        crate::engine::catalog_cache::workspace_names(storage, tenant_id, repo_id).await?;

    // Step 1: expand the spec against the catalog.
    let candidates: Vec<String> = match spec {
        WorkspaceScopeSpec::AllReadable => catalog.as_ref().clone(),
        WorkspaceScopeSpec::Glob(pattern) => {
            let pattern = glob::Pattern::new(pattern)
                .map_err(|e| ExecutionError::Validation(format!("invalid workspace glob: {e}")))?;
            catalog
                .iter()
                .filter(|name| pattern.matches(name))
                .cloned()
                .collect()
        }
        WorkspaceScopeSpec::Exact(names) => names.clone(),
    };

    // Step 2: intersect with what this caller may read.
    let readable = readable_workspaces(auth, &candidates, branch);

    // A caller RLS cannot filter, asking for everything, gets everything -- and
    // gets it with NO filter pushed into the legs, which is also the fast path
    // (no over-fetch, no per-hit permission evaluation). Every internal search
    // takes this branch: indexing jobs, MCP tools, agents running as system.
    if matches!(readable, ReadableWorkspaces::All)
        && matches!(spec, WorkspaceScopeSpec::AllReadable)
    {
        return Ok((
            WorkspaceSet::All,
            ScopeStats {
                catalog: catalog.len(),
                readable: catalog.len(),
            },
        ));
    }

    let in_catalog: BTreeSet<&String> = catalog.iter().collect();
    let mut kept: Vec<String> = Vec::with_capacity(candidates.len());
    for name in &candidates {
        let exists = in_catalog.contains(name);
        let may_read = match &readable {
            ReadableWorkspaces::All => true,
            ReadableWorkspaces::Only(list) => list.contains(name),
        };

        if exists && may_read {
            kept.push(name.clone());
            continue;
        }

        if spec.names_are_assertions() {
            // ONE message for both reasons. The distinction is operator-only.
            tracing::debug!(
                workspace = %name,
                reason = if exists { "not_readable" } else { "not_in_catalog" },
                "search scope: named workspace dropped"
            );
            return Err(ExecutionError::Validation(format!(
                "workspace '{name}' is not available to this query."
            )));
        }

        tracing::debug!(
            workspace = %name,
            reason = if exists { "not_readable" } else { "not_in_catalog" },
            "search scope: pattern match dropped"
        );
    }

    kept.sort();
    kept.dedup();

    let readable_count = match &readable {
        ReadableWorkspaces::All => catalog.len(),
        ReadableWorkspaces::Only(list) => list.len(),
    };
    let stats = ScopeStats {
        catalog: catalog.len(),
        readable: readable_count,
    };

    // Resolving to nothing is zero rows and one INFO line -- never an error, and
    // never a call into an index. "You may read nothing" and "your glob matched
    // nothing" are the same outcome to the caller by design.
    if kept.is_empty() {
        return Ok((WorkspaceSet::Empty, stats));
    }

    Ok((WorkspaceSet::Only(kept), stats))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_and_set() {
        assert_eq!(
            parse_workspace_scope("library").unwrap(),
            WorkspaceScopeSpec::Exact(vec!["library".into()])
        );
        assert_eq!(
            parse_workspace_scope(" library , handbook,policies ").unwrap(),
            WorkspaceScopeSpec::Exact(vec!["library".into(), "handbook".into(), "policies".into()])
        );
    }

    #[test]
    fn all_readable_is_two_words_any_case() {
        assert_eq!(
            parse_workspace_scope("ALL READABLE").unwrap(),
            WorkspaceScopeSpec::AllReadable
        );
        assert_eq!(
            parse_workspace_scope("all readable").unwrap(),
            WorkspaceScopeSpec::AllReadable
        );
    }

    /// `'*'` and bare `'ALL'` must stay rejected, and the error must name the
    /// spelling that works. The whole auditability argument rests on there being
    /// exactly one way to say "broad".
    #[test]
    fn star_and_bare_all_are_rejected_naming_the_alternative() {
        for bad in ["*", "ALL", "all"] {
            let err = parse_workspace_scope(bad).unwrap_err().to_string();
            assert!(err.contains("ALL READABLE"), "unhelpful error: {err}");
        }
        assert!(parse_workspace_scope("").is_err());
        assert!(parse_workspace_scope("   ").is_err());
    }

    #[test]
    fn glob_is_one_token() {
        assert_eq!(
            parse_workspace_scope("content-*").unwrap(),
            WorkspaceScopeSpec::Glob("content-*".into())
        );
        assert!(parse_workspace_scope("a*, b*").is_err());
    }

    /// `Empty` must never look like `All` to a leg.
    #[test]
    fn empty_is_not_all() {
        assert!(WorkspaceSet::Empty.is_empty());
        assert!(!WorkspaceSet::All.is_empty());
        assert_ne!(WorkspaceSet::Empty, WorkspaceSet::All);
        assert_eq!(WorkspaceSet::All.as_filter(), None);
        assert_eq!(
            WorkspaceSet::Only(vec!["a".into()]).as_filter(),
            Some(["a".to_string()].as_slice())
        );
    }
}
