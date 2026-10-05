//! The ONE reference walker RESOLVE uses, and the substitution pass.
//!
//! Both answer "is this a reference?" through [`doc::reference`], so
//! collecting references and replacing them cannot disagree about what a
//! reference is. RESOLVE walks the value it is about to emit — after
//! translation and `fields` trimming — because a reference in a trimmed-away
//! field must not be followed and a reference written by a translation overlay
//! must be. The values are the stored ones (`doc.rs` says why that equals the
//! JSON the resolver used to walk).

use super::budget::{self, ResolveBudget};
use super::doc;
use super::memo::Target;
use raisin_error::Result;
use raisin_models::nodes::properties::PropertyValue;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// A reference as written. `workspace == None` means "the default workspace",
/// which is only known where the reference is used.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct RawRef {
    pub(super) workspace: Option<String>,
    pub(super) locator: String,
}

impl RawRef {
    /// The target this reference names when its default workspace is
    /// `default_workspace`.
    pub(super) fn target(&self, default_workspace: &str) -> TargetRef {
        TargetRef {
            workspace: self
                .workspace
                .clone()
                .unwrap_or_else(|| default_workspace.to_string()),
            locator: self.locator.clone(),
        }
    }
}

/// A fully qualified target: the workspace is part of the identity, so the
/// same path in two workspaces is two targets.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct TargetRef {
    pub(super) workspace: String,
    /// A node id, or a path when it starts with `/` — how references written
    /// into a translation overlay (and by hand) arrive. A node id never starts
    /// with `/`.
    pub(super) locator: String,
}

impl TargetRef {
    pub(super) fn is_path(&self) -> bool {
        self.locator.starts_with('/')
    }
}

/// The distinct references in `value`, in document order.
pub(super) fn distinct_refs(value: &PropertyValue) -> Vec<RawRef> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    collect(value, &mut out, &mut seen);
    out
}

fn collect(value: &PropertyValue, out: &mut Vec<RawRef>, seen: &mut HashSet<RawRef>) {
    if let Some(raw) = RawRef::of(value) {
        if seen.insert(raw.clone()) {
            out.push(raw);
        }
        // A reference is a leaf: its own members are not content.
        return;
    }
    doc::for_each_child(value, &mut |child| collect(child, out, seen));
}

/// What the inlining has produced so far: how many references it replaced
/// (at every nesting level) and roughly how many bytes it inlined.
#[derive(Debug, Default, Clone, Copy)]
pub(super) struct Totals {
    pub(super) occurrences: usize,
    pub(super) bytes: usize,
}

/// Every target a resolution reached: `Some` to inline, `None` to keep bare.
pub(super) type Resolved = HashMap<TargetRef, Option<Arc<Target>>>;

/// The substitution pass. One per resolution.
pub(super) struct Inliner<'a> {
    default_workspace: &'a str,
    resolved: &'a Resolved,
    /// What the statement may still inline; exceeding it is an error.
    allowance: Totals,
    /// The statement's bounds, named in the error.
    limits: ResolveBudget,
    /// Each target expanded once per remaining depth, cloned into every place
    /// it appears.
    /// `None`: expanded once already (and moved into place).
    expanded: HashMap<(TargetRef, u32), Option<(PropertyValue, Totals)>>,
}

impl<'a> Inliner<'a> {
    pub(super) fn new(
        default_workspace: &'a str,
        resolved: &'a Resolved,
        allowance: Totals,
        limits: ResolveBudget,
    ) -> Self {
        Self {
            default_workspace,
            resolved,
            allowance,
            limits,
            expanded: HashMap::new(),
        }
    }

    /// Replace every resolvable reference in `value`, nesting inlined nodes at
    /// most `remaining` levels deep. Returns what was inlined.
    pub(super) fn inline(mut self, value: &mut PropertyValue, remaining: u32) -> Result<Totals> {
        let mut totals = Totals::default();
        self.inline_into(value, remaining, &mut totals)?;
        Ok(totals)
    }

    fn inline_into(
        &mut self,
        value: &mut PropertyValue,
        remaining: u32,
        totals: &mut Totals,
    ) -> Result<()> {
        if let Some((workspace, locator)) = doc::reference(value) {
            if remaining == 0 {
                return Ok(());
            }
            let key = TargetRef {
                workspace: workspace
                    .as_deref()
                    .unwrap_or(self.default_workspace)
                    .to_string(),
                locator: locator.into_owned(),
            };
            let Some(Some(target)) = self.resolved.get(&key) else {
                return Ok(());
            };
            let target = target.clone();
            let (expanded, inner) = self.expand(key, &target, remaining - 1)?;
            totals.occurrences += inner.occurrences + 1;
            totals.bytes += inner.bytes + target.bytes;
            self.check(totals)?;
            *value = expanded;
            return Ok(());
        }
        let mut result = Ok(());
        doc::for_each_child_mut(value, &mut |child| {
            if result.is_ok() {
                result = self.inline_into(child, remaining, totals);
            }
        });
        result
    }

    /// `target` with its own references inlined `remaining` levels deep.
    fn expand(
        &mut self,
        key: TargetRef,
        target: &Arc<Target>,
        remaining: u32,
    ) -> Result<(PropertyValue, Totals)> {
        // A target with nothing to inline expands to itself: copy it from
        // the target, without keeping a second copy in the cache.
        if remaining == 0 || target.refs.is_empty() {
            return Ok((target.value.clone(), Totals::default()));
        }
        // Kept for a SECOND occurrence only: the first expansion is moved into
        // place, so a target inlined once (most of them) is never copied
        // twice; the second one is expanded again and kept, and every later
        // one is a copy of that.
        let cache_key = (key, remaining);
        match self.expanded.get(&cache_key) {
            Some(Some((value, totals))) => return Ok((value.clone(), *totals)),
            Some(None) => {
                let (value, totals) = self.expand_fresh(target, remaining)?;
                self.expanded
                    .insert(cache_key, Some((value.clone(), totals)));
                Ok((value, totals))
            }
            None => {
                self.expanded.insert(cache_key, None);
                self.expand_fresh(target, remaining)
            }
        }
    }

    fn expand_fresh(
        &mut self,
        target: &Arc<Target>,
        remaining: u32,
    ) -> Result<(PropertyValue, Totals)> {
        let mut value = target.value.clone();
        let mut totals = Totals::default();
        self.inline_into(&mut value, remaining, &mut totals)?;
        Ok((value, totals))
    }

    /// Fail as soon as a partial result is already over what the statement may
    /// still inline — a document that fans out exponentially must not be built
    /// in memory first and rejected afterwards.
    fn check(&self, totals: &Totals) -> Result<()> {
        if totals.occurrences > self.allowance.occurrences {
            return Err(budget::occurrences_exceeded(self.limits.max_occurrences));
        }
        if totals.bytes > self.allowance.bytes {
            return Err(budget::bytes_exceeded(self.limits.max_bytes));
        }
        Ok(())
    }
}
