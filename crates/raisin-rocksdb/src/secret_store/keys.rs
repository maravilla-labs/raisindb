//! Key encoding for `cf::SECRETS`.
//!
//! ```text
//! {tenant}\0{repo}\0{branch}\0{name}\0{~rev:16}
//! ```
//!
//! `~rev` is the same 16-byte bitwise-NOT (descending) HLC every versioned CF
//! uses, minted through [`KeyBuilder::push_revision`] rather than by hand — see
//! `keys/node_keys.rs`, whose `path_index` sibling this mirrors exactly.
//!
//! Two properties do all the work:
//!
//! - The descending HLC makes a **forward** prefix scan return a secret's
//!   versions **newest-first**, so `get` reads one entry and stops.
//! - The HLC is a fixed-width trailer, and it MAY CONTAIN NUL BYTES
//!   (`!timestamp_ms` turns every `0xFF` into `0x00`, and `!counter` is all-NUL
//!   at `u64::MAX`). It must therefore be last, located by offset from the end,
//!   and `{name}` must be null-free. Splitting one of these keys on `\0` is the
//!   bug that already cost this codebase live rows in the path index.
//!
//! Being last is also what `BRANCH_CF_REGISTRY`'s `RevisionLocator::Tail`
//! asserts for this CF, which is how the branch copier rewrites the revision on
//! a fork. Do not append a trailing segment.

use raisin_hlc::HLC;

use crate::keys::KeyBuilder;

/// Bytes of a descending HLC trailer.
pub const REV_LEN: usize = 16;

/// One secret version: `{tenant}\0{repo}\0{branch}\0{name}\0{~rev}`.
pub fn secret_key(tenant: &str, repo: &str, branch: &str, name: &str, revision: &HLC) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant)
        .push(repo)
        .push(branch)
        .push(name)
        .push_revision(revision)
        .build()
}

/// Every version of one secret: `{tenant}\0{repo}\0{branch}\0{name}\0`.
pub fn secret_versions_prefix(tenant: &str, repo: &str, branch: &str, name: &str) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant)
        .push(repo)
        .push(branch)
        .push(name)
        .build_prefix()
}

/// Every secret in one branch: `{tenant}\0{repo}\0{branch}\0`.
///
/// The trailing separator is what stops branch `main` from matching `main2`.
pub fn branch_secrets_prefix(tenant: &str, repo: &str, branch: &str) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant)
        .push(repo)
        .push(branch)
        .build_prefix()
}

/// The revision from a secret key: the fixed-width 16-byte trailer.
pub fn revision_from_key(key: &[u8]) -> Option<HLC> {
    let sep = key.len().checked_sub(REV_LEN + 1)?;
    if key[sep] != 0 {
        return None;
    }
    HLC::decode_descending(&key[sep + 1..]).ok()
}

/// The name from a secret key.
///
/// Strips the fixed trailer FIRST, then splits — the trailer's own NUL bytes
/// would otherwise produce a variable number of trailing parts. Because the
/// name is null-free the remainder is exactly four components.
pub fn name_from_key(key: &[u8]) -> Option<String> {
    let sep = key.len().checked_sub(REV_LEN + 1)?;
    if key[sep] != 0 {
        return None;
    }
    let parts: Vec<&[u8]> = key[..sep].split(|&b| b == 0).collect();
    if parts.len() != 4 {
        return None;
    }
    String::from_utf8(parts[3].to_vec()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_layout_is_exactly_the_documented_one() {
        let hlc = HLC::new(1_705_843_009_213, 7);
        let key = secret_key("acme", "site", "main", "node/01H8XY/password", &hlc);
        let mut expected = b"acme\0site\0main\0node/01H8XY/password\0".to_vec();
        expected.extend_from_slice(&hlc.encode_descending());
        assert_eq!(key, expected);
        assert_eq!(key.len() - REV_LEN, expected.len() - REV_LEN);
    }

    #[test]
    fn round_trips_name_and_revision() {
        let hlc = HLC::new(1_705_843_009_213, 7);
        let key = secret_key("t", "r", "main", "a/b.c", &hlc);
        assert_eq!(name_from_key(&key).as_deref(), Some("a/b.c"));
        assert_eq!(revision_from_key(&key), Some(hlc));
    }

    /// The trailer may contain NULs. Locating it by "last separator" — the
    /// mistake the path index shipped — truncates it for ~2% of revisions.
    #[test]
    fn round_trips_when_the_hlc_contains_nul_bytes() {
        let hlc = HLC::new(0x0000_01FF_00FF_FF00, u64::MAX);
        assert!(
            hlc.encode_descending().contains(&0),
            "fixture must have NULs"
        );
        let key = secret_key("t", "r", "main", "stripe", &hlc);
        assert_eq!(name_from_key(&key).as_deref(), Some("stripe"));
        assert_eq!(revision_from_key(&key), Some(hlc));
    }

    /// The whole point of the descending encoding: a FORWARD scan is
    /// newest-first, so `get` reads one entry.
    #[test]
    fn newer_versions_sort_first_under_a_forward_scan() {
        let older = secret_key("t", "r", "main", "k", &HLC::new(1_000, 0));
        let newer = secret_key("t", "r", "main", "k", &HLC::new(2_000, 0));
        let newest = secret_key("t", "r", "main", "k", &HLC::new(2_000, 5));
        let mut keys = vec![older.clone(), newest.clone(), newer.clone()];
        keys.sort();
        assert_eq!(keys, vec![newest, newer, older]);
    }

    #[test]
    fn branch_prefix_does_not_match_a_longer_branch_name() {
        let p = branch_secrets_prefix("t", "r", "main");
        let other = secret_key("t", "r", "main2", "k", &HLC::new(1, 0));
        assert!(!other.starts_with(&p));
        let mine = secret_key("t", "r", "main", "k", &HLC::new(1, 0));
        assert!(mine.starts_with(&p));
    }

    #[test]
    fn version_prefix_does_not_match_a_longer_name() {
        let p = secret_versions_prefix("t", "r", "main", "api");
        assert!(!secret_key("t", "r", "main", "api_key", &HLC::new(1, 0)).starts_with(&p));
        assert!(secret_key("t", "r", "main", "api", &HLC::new(1, 0)).starts_with(&p));
    }
}
