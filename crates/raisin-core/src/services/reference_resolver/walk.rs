//! The ONE JSON reference walker RESOLVE uses, and the substitution pass.
//!
//! Both answer "is this a reference?" through [`as_reference`], so collecting
//! references and replacing them cannot disagree about what a reference is.
//! RESOLVE walks the JSON it is about to emit — after translation and `fields`
//! trimming — because a reference in a trimmed-away field must not be followed
//! and a reference written by a translation overlay must be.

use super::budget::{self, ResolveBudget};
use super::memo::Target;
use raisin_error::Result;
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

const REF_KEY: &str = "raisin:ref";
const WORKSPACE_KEY: &str = "raisin:workspace";

/// A reference as written. `workspace == None` means "the default workspace",
/// which is only known where the reference is used.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct RawRef {
    workspace: Option<String>,
    locator: String,
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

/// `Some((workspace, locator))` when `map` is a reference object.
fn as_reference(map: &Map<String, Value>) -> Option<(Option<&str>, &str)> {
    let locator = map.get(REF_KEY)?.as_str()?;
    let workspace = map
        .get(WORKSPACE_KEY)
        .and_then(Value::as_str)
        .filter(|ws| !ws.is_empty());
    Some((workspace, locator))
}

/// The distinct references in `value`, in document order.
pub(super) fn distinct_refs(value: &Value) -> Vec<RawRef> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    collect(value, &mut out, &mut seen);
    out
}

fn collect(value: &Value, out: &mut Vec<RawRef>, seen: &mut HashSet<RawRef>) {
    match value {
        Value::Object(map) => {
            if let Some((workspace, locator)) = as_reference(map) {
                let raw = RawRef {
                    workspace: workspace.map(str::to_string),
                    locator: locator.to_string(),
                };
                if seen.insert(raw.clone()) {
                    out.push(raw);
                }
                // A reference is a leaf: its own members are not content.
                return;
            }
            for v in map.values() {
                collect(v, out, seen);
            }
        }
        Value::Array(items) => {
            for v in items {
                collect(v, out, seen);
            }
        }
        _ => {}
    }
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
    expanded: HashMap<(TargetRef, u32), (Value, Totals)>,
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
    pub(super) fn inline(mut self, value: &mut Value, remaining: u32) -> Result<Totals> {
        let mut totals = Totals::default();
        self.inline_into(value, remaining, &mut totals)?;
        Ok(totals)
    }

    fn inline_into(
        &mut self,
        value: &mut Value,
        remaining: u32,
        totals: &mut Totals,
    ) -> Result<()> {
        match value {
            Value::Object(map) => {
                if let Some((workspace, locator)) = as_reference(map) {
                    if remaining == 0 {
                        return Ok(());
                    }
                    let key = TargetRef {
                        workspace: workspace.unwrap_or(self.default_workspace).to_string(),
                        locator: locator.to_string(),
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
                for v in map.values_mut() {
                    self.inline_into(v, remaining, totals)?;
                }
            }
            Value::Array(items) => {
                for v in items {
                    self.inline_into(v, remaining, totals)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// `target` with its own references inlined `remaining` levels deep.
    fn expand(
        &mut self,
        key: TargetRef,
        target: &Arc<Target>,
        remaining: u32,
    ) -> Result<(Value, Totals)> {
        let cache_key = (key, remaining);
        if let Some((value, totals)) = self.expanded.get(&cache_key) {
            return Ok((value.clone(), *totals));
        }
        let mut value = target.json.clone();
        let mut totals = Totals::default();
        if remaining > 0 && !target.refs.is_empty() {
            self.inline_into(&mut value, remaining, &mut totals)?;
        }
        self.expanded.insert(cache_key, (value.clone(), totals));
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
