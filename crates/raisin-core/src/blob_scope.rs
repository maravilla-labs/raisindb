//! Which stored blobs a tenant may read through a key it supplied.
//!
//! A blob's storage key lives in node properties — writable content. Anyone
//! who can write a node can therefore point a Resource at ANY key, including
//! another tenant's blob, and then read it back through every path that
//! dereferences the key: an asset download, a signed URL, a package export
//! that packs the bytes. The key itself is the only thing the serve path can
//! check, so these helpers are the check, applied wherever a key that came
//! from a node is read or deleted.
//!
//! Keys are minted as `{tenant}/{yyyy}/{mm}/{dd}/{id}.{ext}`; package build
//! artifacts as `{tenant}/{repo}/pkgbuild/{yyyy}/…`. A key without a tenant
//! segment (`{yyyy}/{mm}/{dd}/{id}`) is the legacy, unprefixed form some
//! writers still produce; it belongs to no tenant provably, so only the
//! lenient check (serving ordinary assets that already exist) admits it.

/// How strictly a key must prove it belongs to the tenant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlobKeyPolicy {
    /// Only `{tenant}/…`. For anything that deletes, and for generated
    /// artifacts (always minted with the prefix).
    Strict,
    /// `{tenant}/…`, or the legacy unprefixed `yyyy/mm/dd/name` form. For
    /// READING an ordinary asset (serving, signing, packing into an export):
    /// unprefixed keys predate tenant prefixes, and refusing them would break
    /// every such asset. A read this admits is one a signed URL of the same
    /// node admits too, so no path reads more than another.
    AllowLegacy,
}

/// A key segment: plain ASCII `[A-Za-z0-9._-]`, not `.`/`..`, not empty.
/// Nothing is decoded or normalised first — `%2e%2e`, a backslash, a colon,
/// an empty segment (`a//b`) or a Unicode look-alike is simply not a
/// segment, so no spelling can be normalised into acceptance.
fn is_segment(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && s.len() <= 255
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// `yyyy/mm/dd/name`, exactly four segments, the date all digits — the
/// unprefixed form writers used before tenant prefixes.
pub fn is_legacy_key(key: &str) -> bool {
    let parts: Vec<&str> = key.split('/').collect();
    parts.len() == 4
        && parts[0].len() == 4
        && parts[1].len() == 2
        && parts[2].len() == 2
        && parts[..3]
            .iter()
            .all(|p| p.bytes().all(|b| b.is_ascii_digit()))
        && is_segment(parts[3])
}

/// Are legacy unprefixed keys readable at all? `RAISIN_LEGACY_BLOB_KEYS=deny`
/// turns them off once a deployment has re-keyed its blobs (see the gap
/// note on legacy keys); the default keeps existing assets working.
pub fn legacy_keys_allowed() -> bool {
    !matches!(
        std::env::var("RAISIN_LEGACY_BLOB_KEYS").as_deref(),
        Ok("deny") | Ok("off") | Ok("false")
    )
}

/// May `tenant` read (or, under [`BlobKeyPolicy::Strict`], delete) the blob
/// at `key`?
///
/// The first segment must EQUAL the tenant (not merely start with it: `acme`
/// does not own `acme-evil/…`), every segment must be a plain segment, and a
/// tenant that is empty or not itself a plain segment owns nothing.
pub fn blob_key_belongs_to(tenant: &str, key: &str, policy: BlobKeyPolicy) -> bool {
    blob_key_belongs_to_with(tenant, key, policy, legacy_keys_allowed())
}

/// [`blob_key_belongs_to`] with the legacy switch made explicit.
pub fn blob_key_belongs_to_with(
    tenant: &str,
    key: &str,
    policy: BlobKeyPolicy,
    allow_legacy: bool,
) -> bool {
    let segments: Vec<&str> = key.split('/').collect();
    if segments.iter().any(|s| !is_segment(s)) {
        return false;
    }
    if is_segment(tenant) && segments.len() >= 2 && segments[0] == tenant {
        return true;
    }
    // A legacy key has a 4-digit year where a tenant-prefixed key has the
    // tenant, and exactly four segments where a prefixed one has at least
    // five — so a legacy key is never another tenant's prefixed key.
    policy == BlobKeyPolicy::AllowLegacy && allow_legacy && is_legacy_key(key)
}

/// The tenant context package builds write their blobs under:
/// `{tenant}/{repo}/pkgbuild`, so an artifact key names its repository.
pub fn artifact_key_namespace(tenant: &str, repo: &str) -> String {
    format!("{tenant}/{repo}/pkgbuild")
}

/// Is `key` a package-build artifact of exactly this tenant and repository?
pub fn is_artifact_key_of(tenant: &str, repo: &str, key: &str) -> bool {
    if !is_segment(tenant) || !is_segment(repo) {
        return false;
    }
    let segments: Vec<&str> = key.split('/').collect();
    segments.len() >= 4
        && segments[0] == tenant
        && segments[1] == repo
        && segments[2] == "pkgbuild"
        && segments.iter().all(|s| is_segment(s))
}

#[cfg(test)]
mod tests {
    use super::*;
    use BlobKeyPolicy::*;

    fn owns(tenant: &str, key: &str, policy: BlobKeyPolicy) -> bool {
        blob_key_belongs_to_with(tenant, key, policy, true)
    }

    #[test]
    fn the_first_segment_must_equal_the_tenant() {
        assert!(owns("acme", "acme/2026/10/11/abc.jpg", Strict));
        for evil in [
            "acme-evil/x.jpg",
            "acme2/x.jpg",
            "acm/x.jpg",
            "evil/acme/x.jpg",
        ] {
            assert!(!owns("acme", evil, Strict), "{evil}");
            assert!(!owns("acme", evil, AllowLegacy), "{evil}");
        }
        assert!(
            !owns("acme", "acme", Strict),
            "the tenant segment alone is no blob"
        );
    }

    #[test]
    fn traversal_encodings_and_odd_segments_are_refused() {
        for key in [
            "acme/../other/x",
            "acme/./x",
            "acme//x",
            "acme/x/",
            "/acme/x",
            "acme/%2e%2e/x",
            "acme/%2E%2E/other",
            "acme\\..\\x",
            "acme/x:y",
            "acme/\u{2215}x",
            "acme/ｘ",
            "",
        ] {
            assert!(!owns("acme", key, Strict), "{key:?}");
            assert!(!owns("acme", key, AllowLegacy), "{key:?}");
        }
    }

    #[test]
    fn an_unusable_tenant_owns_nothing() {
        for tenant in ["", "a/b", "..", ".", "a b", "ａcme"] {
            assert!(!owns(tenant, &format!("{tenant}/x"), Strict), "{tenant:?}");
        }
    }

    #[test]
    fn legacy_keys_follow_a_strict_grammar_and_the_switch() {
        let k = "2025/01/15/abc.png";
        assert!(!owns("acme", k, Strict));
        assert!(owns("acme", k, AllowLegacy));
        assert!(!blob_key_belongs_to_with("acme", k, AllowLegacy, false));
        for not_legacy in [
            "2025/1/15/abc.png",
            "2025/01/15/abc.png/x",
            "2025/01/15",
            "other-tenant-blob-without-prefix-pattern",
            "evil/2025/01/15/abc.png",
            "25/01/15/abc.png",
        ] {
            assert!(!owns("acme", not_legacy, AllowLegacy), "{not_legacy}");
        }
    }

    #[test]
    fn artifacts_name_their_tenant_and_repository_exactly() {
        let k = format!(
            "{}/2026/10/11/x.rap",
            artifact_key_namespace("acme", "site")
        );
        assert!(is_artifact_key_of("acme", "site", &k));
        assert!(!is_artifact_key_of("acme", "other", &k));
        assert!(!is_artifact_key_of("evil", "site", &k));
        assert!(!is_artifact_key_of("acme", "sit", &k));
        assert!(!is_artifact_key_of(
            "acme",
            "site",
            "acme/site-x/pkgbuild/2026/x.rap"
        ));
        assert!(!is_artifact_key_of("acme", "site", "acme/2026/10/11/x.rap"));
        assert!(!is_artifact_key_of(
            "acme",
            "site",
            "acme/site/pkgbuild/../x.rap"
        ));
        assert!(!is_artifact_key_of("acme", "", "acme//pkgbuild/x"));
    }
}
