// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Which account an OIDC assertion is allowed to reach.
//!
//! The rules live here, as a pure function over four facts, so they can be
//! tested without a database. The bug these tests exist for was an ORDERING
//! bug, and ordering is only really covered by playing two logins in sequence.
//!
//! # A verified email is required in BOTH directions
//!
//! Identities are keyed by email, so the email claim decides which account a
//! login reaches. It is only trustworthy when the provider asserts
//! `email_verified`, and the rule has to cover creating an account as well as
//! joining one.
//!
//! Closing only the joining direction leaves **pre-registration takeover**
//! open. That is how this was first written, and what a security review caught.
//!
//! **Read the two-provider version, not the one-provider version.** Anyone
//! relaxing this rule later will picture a single provider, conclude it is a
//! squatting nuisance, and be wrong. The attack that matters crosses two:
//!
//! 1. The attacker persuades a lax provider A to issue an *unverified*
//!    assertion for `victim@example.com`. If creation were allowed, an account
//!    exists for that address, with A's subject pinned to it.
//! 2. The real owner later signs in through provider B with a properly
//!    *verified* assertion for the same address.
//! 3. Step 2 is the joining path, and the email really is verified, so a rule
//!    that only guards joining permits it. Provider B is attached to the
//!    attacker's account.
//!
//! Both parties can now sign in as the same identity, and the attacker was
//! there first. Nothing in step 2 looks wrong from inside the joining rule;
//! the damage was done in step 1.
//!
//! With one provider the outcome is milder but still bad: the attacker keeps
//! the account and the owner is locked out by the subject check below. That
//! milder case is the one that makes this look ignorable. It is not the case
//! the rule is defending against.
//!
//! So an unverified assertion does nothing at all. It cannot create and it
//! cannot join.
//!
//! **Operational consequence, worth knowing before you debug it.** A provider
//! that never sets `email_verified` cannot create accounts here. Some Keycloak
//! realms are configured that way by default. The fix belongs in the provider,
//! by verifying the address or configuring the realm to mark it verified, not
//! in a relaxation here.
//!
//! # What the subject claim adds
//!
//! The subject is the provider's stable identifier for a person. It is recorded
//! the first time we see it and checked on every later login: if a provider
//! issues a different subject for the same address, that is a reassigned
//! mailbox or a reconfigured issuer, and the login is refused rather than
//! quietly re-pointed at an existing account.
//!
//! Finding by email rather than by subject is a deliberate limitation. There is
//! an email index on identities and no subject index, so a person whose
//! provider lets them change their address arrives as a new identity. Adding
//! that index is the fix and it has not been done.

/// What an assertion is allowed to do, decided before anything is written.
///
/// Split out as a pure function so the rules can be tested without a database.
/// The attack above is an ordering bug, and an ordering bug is only really
/// covered by a test that plays the two logins in sequence.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum LinkDecision {
    /// No account has this address; create one.
    Create,
    /// An account exists and this provider may sign in to it.
    Link,
    /// Refuse, with a message safe to return to the caller.
    Refuse(&'static str),
}

/// The email-verification and subject-pinning rules, in one place.
///
/// `existing_subject` is the subject already linked to this account **for this
/// provider**, if any. `None` means the provider has not been used with this
/// account before, which is the joining case.
pub(super) fn decide(
    account_exists: bool,
    existing_subject: Option<&str>,
    presented_subject: &str,
    email_verified: bool,
) -> LinkDecision {
    // A returning user: this provider is already linked to this account. The
    // subject must still match. No verification check, because the link was
    // established under one and re-checking would lock out anyone whose
    // provider stopped sending the claim.
    if let Some(known) = existing_subject {
        return if known == presented_subject {
            LinkDecision::Link
        } else {
            LinkDecision::Refuse(
                "this email is already linked to a different account at this provider",
            )
        };
    }

    // Everything else needs a verified address, whether it creates or joins.
    if !email_verified {
        return LinkDecision::Refuse(
            "the provider did not verify this email address, so it cannot be used to sign in \
             here; verify the address with your provider and try again",
        );
    }

    if account_exists {
        LinkDecision::Link
    } else {
        LinkDecision::Create
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ATTACKER: &str = "attacker-subject";
    const VICTIM: &str = "victim-subject";

    #[test]
    fn a_verified_assertion_creates_an_account() {
        assert_eq!(decide(false, None, VICTIM, true), LinkDecision::Create);
    }

    #[test]
    fn a_verified_assertion_joins_an_existing_account() {
        assert_eq!(decide(true, None, VICTIM, true), LinkDecision::Link);
    }

    #[test]
    fn a_returning_user_signs_in() {
        assert_eq!(decide(true, Some(VICTIM), VICTIM, true), LinkDecision::Link);
    }

    /// A provider that stops sending `email_verified` must not lock out someone
    /// who is already linked. The link was established under a verified
    /// assertion, and the subject still matches.
    #[test]
    fn a_returning_user_is_not_blocked_by_a_missing_verified_claim() {
        assert_eq!(
            decide(true, Some(VICTIM), VICTIM, false),
            LinkDecision::Link
        );
    }

    /// A reassigned mailbox is not the same person.
    #[test]
    fn a_changed_subject_for_a_linked_email_is_refused() {
        assert!(matches!(
            decide(true, Some(VICTIM), ATTACKER, true),
            LinkDecision::Refuse(_)
        ));
    }

    #[test]
    fn an_unverified_assertion_cannot_join_an_existing_account() {
        assert!(matches!(
            decide(true, None, ATTACKER, false),
            LinkDecision::Refuse(_)
        ));
    }

    /// The half that was missing. Refusing only the join left this open, and it
    /// is what makes the sequence below an account takeover.
    #[test]
    fn an_unverified_assertion_cannot_create_an_account() {
        assert!(matches!(
            decide(false, None, ATTACKER, false),
            LinkDecision::Refuse(_)
        ));
    }

    /// Pre-registration takeover, played in order.
    ///
    /// The attacker gets an unverified assertion for the victim's address in
    /// first. If that were allowed to create the account, the victim's later
    /// verified login at any provider would attach to it and the two would
    /// share an identity the attacker already controls. Step one must refuse,
    /// which leaves step two creating a clean account of the victim's own.
    #[test]
    fn arriving_first_with_an_unverified_email_does_not_take_over_the_account() {
        let attacker_first = decide(false, None, ATTACKER, false);
        assert!(
            matches!(attacker_first, LinkDecision::Refuse(_)),
            "an unverified assertion must not create the account"
        );

        // Nothing was written, so the victim still meets an empty namespace.
        let victim_next = decide(false, None, VICTIM, true);
        assert_eq!(
            victim_next,
            LinkDecision::Create,
            "the real owner must get a fresh account, not the attacker's"
        );
    }

    /// The cross-provider shape of the same attack, stated as the invariant it
    /// rests on: whatever an unverified assertion presents, and whether or not
    /// an account already exists, it is refused.
    #[test]
    fn an_unverified_assertion_is_inert_in_every_case() {
        for account_exists in [true, false] {
            assert!(
                matches!(
                    decide(account_exists, None, ATTACKER, false),
                    LinkDecision::Refuse(_)
                ),
                "unverified must be refused with account_exists={account_exists}"
            );
        }
    }
}
