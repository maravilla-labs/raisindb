//! What a local delete or a local move MEANS for the remote, per mount.
//!
//! Two small enums and one resolution rule, kept together because they share
//! that rule: **adapter default < mount override**, and `purge` is never a
//! default. The adapter knows what its provider can do (Drive has a trash, an
//! append-only ledger has neither); the operator knows what this particular
//! mount is for. Neither alone is enough, and hardcoding either in the engine
//! would put provider knowledge in the domain-blind half of the system.
//!
//! There is deliberately **no `move` operation**. A move is an `update` carrying
//! the new parent/folder field, so `move_policy: push` means "include that field
//! in the update" and nothing more. Adding a fifth verb to the adapter contract
//! to express a special case of the fourth is how op surfaces grow without
//! bound.

use super::super::{Capabilities, WriteConfig};

/// What a local delete does to the remote object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeletePolicy {
    /// Nothing is pushed. The node goes locally and the remote is untouched.
    ///
    /// **This is not "the delete was suppressed".** There is no per-mount
    /// suppression set anywhere in the engine, so the item is still at the
    /// provider, still reported by `list` / `get_changes`, and the next full
    /// reconcile — or the next delta that mentions it — **re-imports it**. The
    /// node comes back. That is inherent to detaching rather than deleting, and
    /// documenting it as anything else generates bug reports.
    Detach,
    /// Push a soft delete: the provider's trash / recycle bin, recoverable by a
    /// human. Requires the adapter to declare `supports_trash`.
    Trash,
    /// Push a hard, irreversible delete.
    ///
    /// Never a default at any layer — not the engine's, not an adapter's. An
    /// operator has to type it.
    Purge,
}

impl DeletePolicy {
    /// Whether this policy reaches the provider at all.
    pub(crate) fn pushes(&self) -> bool {
        !matches!(self, Self::Detach)
    }

    /// The wire value handed to the adapter's `delete` operation.
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Detach => "detach",
            Self::Trash => "trash",
            Self::Purge => "purge",
        }
    }

    fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "detach" => Some(Self::Detach),
            "trash" => Some(Self::Trash),
            "purge" => Some(Self::Purge),
            _ => None,
        }
    }
}

/// What a local move (a re-parent, i.e. a changed folder field) does remotely.
///
/// This governs a move WITHIN the mount — a changed value of a field the
/// adapter declared in [`Capabilities::move_fields`]. A node moved OUT of the
/// mount path entirely is a different event with no remote counterpart (there
/// is no provider folder for "outside this mount"), and is always a detach; see
/// [`super::relocate`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MovePolicy {
    /// Include the folder/parent field in the outbound `update`.
    Push,
    /// Leave the remote where it is: the move field is dropped from the
    /// outbound update, and nothing else about the node changes.
    ///
    /// **The local move still sticks.** The read path preserves an existing
    /// node's path and its stored `__pushed_state` on upsert, and a relocation
    /// only happens under an operator-triggered remap (`force_rewrite`), so
    /// nothing moves the node back. The mount simply diverges from the provider
    /// on that one field, quietly and permanently, which is what an operator
    /// choosing `detach` is asking for. Describing it as "until the next
    /// reconcile moves it back" — as this doc did — was wrong in the direction
    /// that generates bug reports.
    Detach,
    /// Refuse the write: the whole update is withheld while a move field
    /// diverges, and the reason reaches `writeback_last_error`.
    ///
    /// Refusing the WHOLE update rather than the move field alone is
    /// deliberate. A half-applied node — new flags at the provider, old folder
    /// — is a state neither the user nor the operator asked for, and it is
    /// indistinguishable at the provider from a successful push. The local move
    /// still sticks (see [`Self::Detach`]); what `reject` buys is that the
    /// divergence is stated out loud on every drain instead of being silent.
    Reject,
}

impl MovePolicy {
    /// Whether a declared move field travels with the outbound update.
    pub(crate) fn pushes(&self) -> bool {
        matches!(self, Self::Push)
    }

    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Push => "push",
            Self::Detach => "detach",
            Self::Reject => "reject",
        }
    }

    fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "push" => Some(Self::Push),
            "detach" => Some(Self::Detach),
            "reject" => Some(Self::Reject),
            _ => None,
        }
    }
}

/// The engine's own fallback when neither the adapter nor the mount says
/// anything.
///
/// `Detach` — the reading that changes nothing at the provider. The precedent is
/// `allow_empty_reconcile`: where a configuration is ambiguous, the destructive
/// reading is never the default, because stale remote content is recoverable and
/// deleted remote content is not.
const ENGINE_DEFAULT_DELETE: DeletePolicy = DeletePolicy::Detach;
const ENGINE_DEFAULT_MOVE: MovePolicy = MovePolicy::Detach;

/// Resolve the effective delete policy: engine default < adapter default <
/// mount override.
///
/// Returns `Err` with an operator-facing reason when the resolved policy cannot
/// be honoured, which is a REFUSAL rather than a silent downgrade. Two cases:
/// an unrecognized string (a typo must never fall back to a policy the operator
/// did not choose) and `trash` on an adapter that has no trash (silently
/// promoting that to `purge` would turn a recoverable delete into an
/// irreversible one — the single worst substitution this module could make).
///
/// An adapter declaring `default_delete_policy: "purge"` is IGNORED, with a
/// warning: `purge` is never a default. An adapter's opinion about the safest
/// behaviour is worth taking; its opinion that irreversible deletion is the
/// safest behaviour is not.
pub(crate) fn resolve_delete_policy(
    write_config: &WriteConfig,
    capabilities: &Capabilities,
) -> Result<DeletePolicy, String> {
    let mut policy = ENGINE_DEFAULT_DELETE;
    if let Some(raw) = capabilities.default_delete_policy.as_deref() {
        match DeletePolicy::parse(raw) {
            Some(DeletePolicy::Purge) => {
                tracing::warn!(
                    adapter_default = %raw,
                    "adapter declares `purge` as its default delete policy; ignoring it. \
                     An irreversible delete is never a default — set delete_policy on the \
                     mount to opt in."
                );
            }
            Some(parsed) => policy = parsed,
            None => {
                return Err(format!(
                    "adapter declares an unrecognized default_delete_policy '{raw}'; \
                     expected 'detach', 'trash' or 'purge'"
                ))
            }
        }
    }
    if let Some(raw) = write_config
        .delete_policy
        .as_deref()
        .filter(|s| !s.is_empty())
    {
        policy = DeletePolicy::parse(raw).ok_or_else(|| {
            format!(
                "write_config.delete_policy '{raw}' is not recognized; \
                 expected 'detach', 'trash' or 'purge'"
            )
        })?;
    }
    if policy == DeletePolicy::Trash && !capabilities.supports_trash {
        return Err(
            "delete_policy is 'trash' but the adapter does not declare supports_trash; \
             refusing rather than silently purging"
                .to_string(),
        );
    }
    Ok(policy)
}

/// Resolve the effective move policy, same precedence as the delete one.
pub(crate) fn resolve_move_policy(
    write_config: &WriteConfig,
    capabilities: &Capabilities,
) -> Result<MovePolicy, String> {
    let mut policy = ENGINE_DEFAULT_MOVE;
    if let Some(raw) = capabilities.default_move_policy.as_deref() {
        policy = MovePolicy::parse(raw).ok_or_else(|| {
            format!(
                "adapter declares an unrecognized default_move_policy '{raw}'; \
                 expected 'push', 'detach' or 'reject'"
            )
        })?;
    }
    if let Some(raw) = write_config
        .move_policy
        .as_deref()
        .filter(|s| !s.is_empty())
    {
        policy = MovePolicy::parse(raw).ok_or_else(|| {
            format!(
                "write_config.move_policy '{raw}' is not recognized; \
                 expected 'push', 'detach' or 'reject'"
            )
        })?;
    }
    Ok(policy)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps(default_delete: Option<&str>, trash: bool) -> Capabilities {
        Capabilities {
            default_delete_policy: default_delete.map(str::to_string),
            supports_trash: trash,
            ..Default::default()
        }
    }

    fn wc(delete: Option<&str>) -> WriteConfig {
        WriteConfig {
            delete_policy: delete.map(str::to_string),
            ..Default::default()
        }
    }

    /// Nothing configured anywhere means nothing is pushed. The destructive
    /// reading is never the default.
    #[test]
    fn the_engine_default_touches_no_remote() {
        let p = resolve_delete_policy(&wc(None), &caps(None, false)).unwrap();
        assert_eq!(p, DeletePolicy::Detach);
        assert!(!p.pushes());
    }

    /// Adapter default < mount override.
    #[test]
    fn the_mount_overrides_the_adapters_default() {
        let p = resolve_delete_policy(&wc(Some("trash")), &caps(Some("detach"), true)).unwrap();
        assert_eq!(p, DeletePolicy::Trash);
    }

    /// `purge` is never a default, at any layer. An adapter that declares it is
    /// ignored; an operator who types it on the mount gets it.
    #[test]
    fn purge_is_never_a_default_but_is_always_available_explicitly() {
        assert_eq!(
            resolve_delete_policy(&wc(None), &caps(Some("purge"), false)).unwrap(),
            DeletePolicy::Detach,
            "an adapter cannot make irreversible deletion the default"
        );
        assert_eq!(
            resolve_delete_policy(&wc(Some("purge")), &caps(None, false)).unwrap(),
            DeletePolicy::Purge
        );
    }

    /// `trash` on an adapter with no trash is REFUSED, never promoted to
    /// `purge`. Silently substituting an irreversible delete for a recoverable
    /// one is the single worst thing this resolution could do.
    #[test]
    fn trash_without_adapter_support_is_refused_not_promoted() {
        let e = resolve_delete_policy(&wc(Some("trash")), &caps(None, false)).unwrap_err();
        assert!(e.contains("supports_trash"), "{e}");
    }

    /// A typo is a refusal, not a fallback: falling back would apply a policy
    /// the operator never chose, and the one they meant to type may have been
    /// `detach`.
    #[test]
    fn an_unrecognized_policy_is_refused() {
        assert!(resolve_delete_policy(&wc(Some("delete")), &caps(None, true)).is_err());
        assert!(resolve_move_policy(
            &WriteConfig {
                move_policy: Some("moove".into()),
                ..Default::default()
            },
            &caps(None, true)
        )
        .is_err());
    }
}
