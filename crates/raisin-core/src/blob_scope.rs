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

fn is_segment(s: &str) -> bool {
    !s.is_empty() && s != "." && s != ".." && !s.contains('\\') && !s.contains(':')
}

/// `yyyy/mm/dd/name`, exactly four segments, the date all digits.
fn is_legacy_key(key: &str) -> bool {
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

/// May `tenant` read (or, under [`BlobKeyPolicy::Strict`], delete) the blob
/// at `key`?
pub fn blob_key_belongs_to(tenant: &str, key: &str, policy: BlobKeyPolicy) -> bool {
    if key.starts_with('/') || key.split('/').any(|s| !is_segment(s)) {
        return false;
    }
    if is_segment(tenant) && !tenant.contains('/') {
        if let Some(rest) = key.strip_prefix(tenant) {
            if rest.starts_with('/') && rest.len() > 1 {
                return true;
            }
        }
    }
    policy == BlobKeyPolicy::AllowLegacy && is_legacy_key(key)
}

/// The tenant context package builds write their blobs under:
/// `{tenant}/{repo}/pkgbuild`, so an artifact key names its repository.
pub fn artifact_key_namespace(tenant: &str, repo: &str) -> String {
    format!("{tenant}/{repo}/pkgbuild")
}

/// Is `key` a package-build artifact of exactly this tenant and repository?
pub fn is_artifact_key_of(tenant: &str, repo: &str, key: &str) -> bool {
    if !is_segment(tenant) || !is_segment(repo) || tenant.contains('/') || repo.contains('/') {
        return false;
    }
    let ns = format!("{}/", artifact_key_namespace(tenant, repo));
    key.starts_with(&ns)
        && key.len() > ns.len()
        && blob_key_belongs_to(tenant, key, BlobKeyPolicy::Strict)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tenant_owns_only_its_prefix() {
        let k = "acme/2026/10/11/abc.jpg";
        assert!(blob_key_belongs_to("acme", k, BlobKeyPolicy::Strict));
        assert!(!blob_key_belongs_to("evil", k, BlobKeyPolicy::Strict));
        assert!(!blob_key_belongs_to("evil", k, BlobKeyPolicy::AllowLegacy));
        // A prefix of the tenant name is not the tenant.
        assert!(!blob_key_belongs_to("acm", k, BlobKeyPolicy::Strict));
        assert!(!blob_key_belongs_to("acme", "acme/", BlobKeyPolicy::Strict));
        assert!(!blob_key_belongs_to(
            "acme",
            "acme/../evil/x",
            BlobKeyPolicy::Strict
        ));
        assert!(!blob_key_belongs_to(
            "acme",
            "/acme/x",
            BlobKeyPolicy::Strict
        ));
        assert!(!blob_key_belongs_to("", "x/y", BlobKeyPolicy::Strict));
    }

    #[test]
    fn legacy_keys_only_under_the_lenient_policy() {
        let k = "2025/01/15/abc.png";
        assert!(!blob_key_belongs_to("acme", k, BlobKeyPolicy::Strict));
        assert!(blob_key_belongs_to("acme", k, BlobKeyPolicy::AllowLegacy));
        // Another tenant's key is never "legacy".
        assert!(!blob_key_belongs_to(
            "acme",
            "evil/2025/01/15/abc.png",
            BlobKeyPolicy::AllowLegacy
        ));
        assert!(!blob_key_belongs_to(
            "acme",
            "2025/1/15/abc.png",
            BlobKeyPolicy::AllowLegacy
        ));
    }

    #[test]
    fn artifacts_name_their_repository() {
        let k = format!(
            "{}/2026/10/11/x.rap",
            artifact_key_namespace("acme", "site")
        );
        assert!(is_artifact_key_of("acme", "site", &k));
        assert!(!is_artifact_key_of("acme", "other", &k));
        assert!(!is_artifact_key_of("evil", "site", &k));
        assert!(!is_artifact_key_of("acme", "site", "acme/2026/10/11/x.rap"));
    }
}
