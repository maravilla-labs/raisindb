// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! What a function may pass where a secret is named.
//!
//! `raisin.secrets.get()` accepts **either** spelling of the same secret:
//!
//! - a bare name — `"stripe_key"`, `"node/01H8XY/api_key"`
//! - a full reference — `"secret://node/01H8XY/api_key@1"`
//!
//! Both are accepted because the primary use case hands the caller the second
//! one. A node property in a field declared `encrypted: true` stores a
//! reference, and node reads never resolve, so the function receives the whole
//! `secret://…@N` string and must be able to pass it straight back:
//!
//! ```javascript
//! const node = await raisin.nodes.get('data', '/connections/stripe');
//! const key  = raisin.secrets.get(node.properties.api_key);
//! ```
//!
//! # Parsing is delegated, never re-implemented
//!
//! [`raisin_models::secret_ref::SecretRef::parse`] owns the rule, including the
//! subtle part: a name MAY contain `@` (`ops@example.com`), so only a non-empty
//! all-digit run after the LAST `@` is a version. Hand-rolling that here — or
//! in a function author's own code, which is what a name-only API would force —
//! gets `secret://a@beta` wrong. Everything below funnels through that parser.
//!
//! # One secret, one answer
//!
//! Both spellings reduce to the same [`SecretSpec::name`] BEFORE the policy is
//! consulted, so `SecretPolicy` cannot be dodged by re-spelling: a function
//! granted `stripe_key` gets the same allow/deny answer for `"stripe_key"` and
//! for `"secret://stripe_key@2"`.

use raisin_error::Result;
use raisin_models::secret_ref::{validate_secret_name, SecretRef};

/// A resolved `{ name, version }` pair, whichever spelling produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretSpec {
    /// The store name. This is what the policy allow-list matches against.
    pub name: String,
    /// `None` reads the newest version.
    pub version: Option<u64>,
}

fn invalid(spec: &str, e: impl std::fmt::Display) -> raisin_error::Error {
    raisin_error::Error::Validation(format!("invalid secret name or reference {spec:?}: {e}"))
}

/// Parse either spelling into `{ name, version }`.
///
/// A bare name never carries a version; a reference carries whatever it pinned.
pub fn parse_secret_spec(spec: &str) -> Result<SecretSpec> {
    if SecretRef::is_secret_ref(spec) {
        let parsed = SecretRef::parse(spec).map_err(|e| invalid(spec, e))?;
        return Ok(SecretSpec {
            name: parsed.name,
            version: parsed.version,
        });
    }
    validate_secret_name(spec).map_err(|e| invalid(spec, e))?;
    Ok(SecretSpec {
        name: spec.to_string(),
        version: None,
    })
}

/// The read form: a spec plus the optional explicit `version` argument.
///
/// A version pinned INSIDE the reference is honoured — that is what keeps a
/// time-travel read honest, since an older node revision holds an older
/// version and must resolve to what the node actually held then.
///
/// # A pinned reference plus an explicit version is an ERROR
///
/// `get("secret://k@1", 2)` states two different intents, and there is no
/// resolution that does not silently discard one of them. Picking either is
/// worse than refusing, because of the case this API exists to protect: a
/// caller reads an OLD node revision, whose property is pinned to the version
/// that revision actually held, then passes a version from somewhere else. One
/// choice returns a value the node never held; the other makes the argument
/// look ignored. Both are silent, and the pinned reference exists precisely to
/// guarantee neither can happen.
///
/// Nobody writes both deliberately, so refusing costs nothing and converts a
/// silent wrong answer into a loud one. A bare name plus a version, and an
/// UNPINNED reference plus a version, both keep working — the argument is the
/// only version stated in either case.
pub fn parse_read_spec(spec: &str, explicit_version: Option<i64>) -> Result<SecretSpec> {
    let mut parsed = parse_secret_spec(spec)?;
    if let Some(v) = explicit_version {
        let v = u64::try_from(v).map_err(|_| {
            raisin_error::Error::Validation(format!(
                "secret version must be a positive ordinal, got {v}"
            ))
        })?;
        if let Some(pinned) = parsed.version {
            return Err(raisin_error::Error::Validation(format!(
                "conflicting versions for secret '{}': the reference {spec:?} pins version                  {pinned}, but version {v} was passed as an argument. Pass one or the other —                  a pinned reference already says which version to read, and silently                  preferring either would hide the disagreement.",
                parsed.name
            )));
        }
        parsed.version = Some(v);
    }
    Ok(parsed)
}

/// The write form: a name, with a pinned reference REFUSED.
///
/// Every write appends a new version, so `put`/`rotate`/`delete` targeting
/// `secret://k@1` cannot mean what it says. Refusing is the honest answer;
/// accepting and ignoring the pin would let an author believe they had
/// overwritten version 1.
pub fn parse_write_spec(spec: &str, op: &str) -> Result<String> {
    let parsed = parse_secret_spec(spec)?;
    if let Some(v) = parsed.version {
        return Err(raisin_error::Error::Validation(format!(
            "cannot {op} the pinned reference {spec:?}: writes always append a new version, \
             they never replace version {v}. Pass the name (or an unpinned \
             secret://{}) instead.",
            parsed.name
        )));
    }
    Ok(parsed.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_bare_name() {
        let s = parse_secret_spec("stripe_key").unwrap();
        assert_eq!(s.name, "stripe_key");
        assert_eq!(s.version, None);
    }

    #[test]
    fn accepts_a_full_reference_and_honours_its_pin() {
        let s = parse_secret_spec("secret://node/01H8XY/api_key@1").unwrap();
        assert_eq!(s.name, "node/01H8XY/api_key");
        assert_eq!(s.version, Some(1));
    }

    #[test]
    fn both_spellings_reduce_to_the_same_name() {
        // The property that makes the policy un-dodgeable.
        assert_eq!(
            parse_secret_spec("stripe_key").unwrap().name,
            parse_secret_spec("secret://stripe_key@7").unwrap().name
        );
    }

    /// The `@` rule a function author would get wrong by hand: a name may
    /// contain `@`, so only a trailing all-digit run is a version.
    #[test]
    fn an_at_sign_in_the_name_is_not_a_version() {
        let s = parse_secret_spec("secret://ops@example.com").unwrap();
        assert_eq!(s.name, "ops@example.com");
        assert_eq!(s.version, None);

        let s = parse_secret_spec("secret://a@beta").unwrap();
        assert_eq!(s.name, "a@beta");
        assert_eq!(s.version, None);
    }

    /// Two stated versions is unresolvable, so it is refused rather than
    /// silently resolved — see `parse_read_spec`.
    #[test]
    fn a_pin_plus_an_explicit_version_is_an_error() {
        let err = parse_read_spec("secret://k@1", Some(3)).unwrap_err();
        let msg = err.to_string();
        // The message must name BOTH versions, or the caller cannot see which
        // of the two things they wrote is the one to remove.
        assert!(msg.contains('1'), "should name the pinned version: {msg}");
        assert!(msg.contains('3'), "should name the argument version: {msg}");
        assert!(msg.contains('k'), "should name the secret: {msg}");
    }

    /// Only ONE version is stated in each of these, so they are unambiguous
    /// and must keep working.
    #[test]
    fn a_single_stated_version_still_works() {
        // Pin only.
        assert_eq!(
            parse_read_spec("secret://k@1", None).unwrap().version,
            Some(1)
        );
        // Argument only, bare name.
        assert_eq!(parse_read_spec("k", Some(2)).unwrap().version, Some(2));
        // Argument only, UNPINNED reference.
        assert_eq!(
            parse_read_spec("secret://k", Some(2)).unwrap().version,
            Some(2)
        );
        // Neither: newest.
        assert_eq!(parse_read_spec("secret://k", None).unwrap().version, None);
    }

    #[test]
    fn rejects_unusable_names() {
        assert!(parse_secret_spec("").is_err());
        assert!(parse_secret_spec("a b").is_err());
        assert!(parse_secret_spec("secret://").is_err());
    }

    #[test]
    fn a_write_refuses_a_pinned_reference() {
        let err = parse_write_spec("secret://k@1", "rotate").unwrap_err();
        assert!(
            err.to_string().contains("append"),
            "message should explain why: {err}"
        );
        // Unpinned forms are fine.
        assert_eq!(parse_write_spec("secret://k", "rotate").unwrap(), "k");
        assert_eq!(parse_write_spec("k", "rotate").unwrap(), "k");
    }
}
