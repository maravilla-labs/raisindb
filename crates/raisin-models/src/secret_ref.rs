// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! `secret://` references — the value a node property actually holds.
//!
//! A property in a vaulted field never stores ciphertext. It stores a
//! **reference** to an entry in the secret store:
//!
//! ```text
//! secret://<name>            -- resolves to the newest version
//! secret://<name>@<version>  -- pins one version
//! ```
//!
//! # Why this lives in `raisin-models`
//!
//! The store itself is a module inside `raisin-rocksdb` (it writes a column
//! family), but *recognising* a reference is something the SQL layer, the
//! transports, the functions runtime and the console all need to do without
//! pulling in the storage engine. Parsing therefore lives here, at the bottom
//! of the dependency graph, next to the other property-shaped types.
//!
//! # Name rules
//!
//! A name may contain `/` — the store's own convention for a vaulted schema
//! field is `node/{node_id}/{field.path}`. It may **not** contain:
//!
//! - `\0`, because the storage key is `{tenant}\0{repo}\0{branch}\0{name}\0{~rev}`
//!   and the descending HLC trailer is located by fixed offset from the end. A
//!   NUL inside the name would make the key un-parseable.
//! - whitespace or other control characters, because a reference appears inside
//!   JSON, URLs and log lines where a stray space silently truncates it.
//!
//! # The `@` rule
//!
//! A name is allowed to contain `@` (e.g. an email-shaped operator name). The
//! version suffix is therefore recognised only when the text after the **last**
//! `@` is a non-empty run of ASCII digits. `secret://a@b` is the name `a@b`;
//! `secret://a@2` is the name `a` at version 2. A name that both contains `@`
//! and ends in `@<digits>` is ambiguous by construction — [`format_secret_ref`]
//! never produces one, and [`SecretRef::parse`] resolves it as a version.

use std::fmt;

use serde::{Deserialize, Serialize};

/// The URI scheme, including the `://`.
pub const SECRET_SCHEME: &str = "secret://";

/// Why a `secret://` string could not be parsed, or a name could not be used.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SecretRefError {
    /// The string does not begin with `secret://`.
    #[error("not a secret reference (expected a `secret://` prefix): {0:?}")]
    NotASecretRef(String),

    /// The name part was empty.
    #[error("secret reference has an empty name")]
    EmptyName,

    /// The name contained a NUL byte. This is the one that would corrupt the
    /// storage key, so it is rejected at every entry point.
    #[error("secret name must not contain a NUL byte: {0:?}")]
    NulInName(String),

    /// The name contained whitespace or another control character.
    #[error("secret name must not contain whitespace or control characters: {0:?}")]
    WhitespaceInName(String),

    /// The `@<version>` suffix was present but not a valid `u64`.
    #[error("secret reference version is not a valid ordinal: {0:?}")]
    BadVersion(String),
}

/// A parsed `secret://` reference.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SecretRef {
    /// The store name. Null-free, whitespace-free; may contain `/`.
    pub name: String,
    /// `None` means "whatever is newest at read time".
    pub version: Option<u64>,
}

impl SecretRef {
    /// A reference to the newest version of `name`.
    pub fn latest(name: impl Into<String>) -> Result<Self, SecretRefError> {
        let name = name.into();
        validate_secret_name(&name)?;
        Ok(Self {
            name,
            version: None,
        })
    }

    /// A reference pinned to one version ordinal.
    pub fn pinned(name: impl Into<String>, version: u64) -> Result<Self, SecretRefError> {
        let name = name.into();
        validate_secret_name(&name)?;
        Ok(Self {
            name,
            version: Some(version),
        })
    }

    /// Parse `secret://<name>` or `secret://<name>@<version>`.
    pub fn parse(s: &str) -> Result<Self, SecretRefError> {
        let rest = s
            .strip_prefix(SECRET_SCHEME)
            .ok_or_else(|| SecretRefError::NotASecretRef(s.to_string()))?;

        // Only a trailing all-digit run after the LAST `@` is a version; see the
        // module docs for why a name may otherwise contain `@`.
        let (name, version) = match rest.rsplit_once('@') {
            Some((head, tail)) if !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit()) => {
                let v = tail
                    .parse::<u64>()
                    .map_err(|_| SecretRefError::BadVersion(tail.to_string()))?;
                (head, Some(v))
            }
            _ => (rest, None),
        };

        validate_secret_name(name)?;
        Ok(Self {
            name: name.to_string(),
            version,
        })
    }

    /// Whether `s` looks like a secret reference at all. Cheap enough to call
    /// on every property value.
    pub fn is_secret_ref(s: &str) -> bool {
        s.starts_with(SECRET_SCHEME)
    }
}

impl fmt::Display for SecretRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.version {
            Some(v) => write!(f, "{SECRET_SCHEME}{}@{v}", self.name),
            None => write!(f, "{SECRET_SCHEME}{}", self.name),
        }
    }
}

/// Format a reference without building a [`SecretRef`].
pub fn format_secret_ref(name: &str, version: Option<u64>) -> String {
    match version {
        Some(v) => format!("{SECRET_SCHEME}{name}@{v}"),
        None => format!("{SECRET_SCHEME}{name}"),
    }
}

/// The single definition of a legal secret name.
///
/// Called by the reference parser *and* by the store's write path, so the two
/// cannot drift into accepting different name sets.
pub fn validate_secret_name(name: &str) -> Result<(), SecretRefError> {
    if name.is_empty() {
        return Err(SecretRefError::EmptyName);
    }
    if name.contains('\0') {
        return Err(SecretRefError::NulInName(name.to_string()));
    }
    if name.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(SecretRefError::WhitespaceInName(name.to_string()));
    }
    Ok(())
}

/// The store name convention for a vaulted schema field on a node.
///
/// `node/{node_id}/{field_path}` — `field_path` is the dot path the property
/// walker produces, so a nested or element field gets a distinct secret.
pub fn node_field_secret_name(node_id: &str, field_path: &str) -> String {
    format!("node/{node_id}/{field_path}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_latest_and_pinned() {
        let r = SecretRef::parse("secret://stripe_key").unwrap();
        assert_eq!(r.name, "stripe_key");
        assert_eq!(r.version, None);

        let r = SecretRef::parse("secret://stripe_key@3").unwrap();
        assert_eq!(r.name, "stripe_key");
        assert_eq!(r.version, Some(3));
    }

    #[test]
    fn round_trips_through_display() {
        for s in [
            "secret://stripe_key",
            "secret://stripe_key@3",
            "secret://node/01H8XY/password",
            "secret://node/01H8XY/password@12",
            "secret://node/01H8XY/venue.geo.token@1",
            "secret://ops@example.com",
        ] {
            let parsed = SecretRef::parse(s).unwrap();
            assert_eq!(parsed.to_string(), s, "round trip failed for {s}");
            assert_eq!(SecretRef::parse(&parsed.to_string()).unwrap(), parsed);
        }
    }

    #[test]
    fn format_helper_matches_display() {
        assert_eq!(
            format_secret_ref("a/b", Some(7)),
            SecretRef::pinned("a/b", 7).unwrap().to_string()
        );
        assert_eq!(
            format_secret_ref("a/b", None),
            SecretRef::latest("a/b").unwrap().to_string()
        );
    }

    #[test]
    fn a_name_may_contain_slashes_and_at_signs() {
        let r = SecretRef::parse("secret://ops@example.com").unwrap();
        assert_eq!(r.name, "ops@example.com");
        assert_eq!(r.version, None);
    }

    #[test]
    fn rejects_missing_scheme() {
        assert!(matches!(
            SecretRef::parse("stripe_key"),
            Err(SecretRefError::NotASecretRef(_))
        ));
        assert!(matches!(
            SecretRef::parse("https://example.com"),
            Err(SecretRefError::NotASecretRef(_))
        ));
    }

    #[test]
    fn rejects_empty_nul_and_whitespace_names() {
        assert!(matches!(
            SecretRef::parse("secret://"),
            Err(SecretRefError::EmptyName)
        ));
        assert!(matches!(
            SecretRef::latest("a\0b"),
            Err(SecretRefError::NulInName(_))
        ));
        assert!(matches!(
            SecretRef::parse("secret://a b"),
            Err(SecretRefError::WhitespaceInName(_))
        ));
        assert!(matches!(
            SecretRef::parse("secret://a\tb"),
            Err(SecretRefError::WhitespaceInName(_))
        ));
    }

    #[test]
    fn non_numeric_suffix_is_part_of_the_name() {
        let r = SecretRef::parse("secret://a@beta").unwrap();
        assert_eq!(r.name, "a@beta");
        assert_eq!(r.version, None);
    }

    #[test]
    fn is_secret_ref_discriminates() {
        assert!(SecretRef::is_secret_ref("secret://x"));
        assert!(!SecretRef::is_secret_ref("secrets://x"));
        assert!(!SecretRef::is_secret_ref("x"));
    }

    #[test]
    fn node_field_name_convention() {
        assert_eq!(
            node_field_secret_name("01H8XY", "venue.token"),
            "node/01H8XY/venue.token"
        );
        assert!(validate_secret_name(&node_field_secret_name("01H8XY", "venue.token")).is_ok());
    }
}
