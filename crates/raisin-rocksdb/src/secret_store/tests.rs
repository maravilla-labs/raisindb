//! Secret store tests.
//!
//! Every test runs with `RAISIN_CRYPTO_EMIT_V2` set. That is not a convenience:
//! the node-secret crypto family is `V1Policy::Reject`, so a v1 blob written
//! here could never be read back, and [`SecretError::V2EmissionRequired`] is
//! what the store returns instead of writing one. The variable is only ever
//! *set*, never cleared, so parallel tests in this binary cannot race on it.

use std::sync::{Arc, Once};

use raisin_crypto::{KeyProvider, Keyring, SecretBox};
use rocksdb::{Options, DB};
use tempfile::TempDir;

use super::*;
use crate::cf;

static EMIT_V2: Once = Once::new();

fn enable_v2_emission() {
    EMIT_V2.call_once(|| std::env::set_var("RAISIN_CRYPTO_EMIT_V2", "1"));
}

fn store_with_keyring(keys: Arc<dyn KeyProvider>) -> (TempDir, SecretStore) {
    enable_v2_emission();
    let dir = TempDir::new().unwrap();
    let mut opts = Options::default();
    opts.create_if_missing(true);
    opts.create_missing_column_families(true);
    let db = DB::open_cf(&opts, dir.path(), vec![cf::SECRETS]).unwrap();
    let store = SecretStore::new(
        Arc::new(db),
        Arc::new(SecretBox::with_keyring(keys)),
        "test-node",
    );
    (dir, store)
}

fn test_store() -> (TempDir, SecretStore) {
    store_with_keyring(Arc::new(Keyring::new(vec![(1, [1u8; 32])], 1).unwrap()))
}

fn scope() -> SecretScope {
    SecretScope::new("acme", "site", "main")
}

fn owner() -> SecretOwner {
    SecretOwner::actor("alice")
}

// ---- round trip ---------------------------------------------------------

#[test]
fn put_then_get_round_trips() {
    let (_d, store) = test_store();
    let (name, version) = store
        .put(&scope(), "stripe_key", b"sk_live_abc", &owner())
        .unwrap();
    assert_eq!(name, "stripe_key");
    assert_eq!(version, 1);
    assert_eq!(
        store.get(&scope(), "stripe_key", None).unwrap(),
        b"sk_live_abc"
    );
}

#[test]
fn two_puts_give_two_versions_and_get_returns_the_newest() {
    let (_d, store) = test_store();
    let s = scope();
    assert_eq!(store.put(&s, "k", b"first", &owner()).unwrap().1, 1);
    assert_eq!(store.put(&s, "k", b"second", &owner()).unwrap().1, 2);
    assert_eq!(store.get(&s, "k", None).unwrap(), b"second");
    assert_eq!(store.list_versions(&s, "k").unwrap().len(), 2);
    // Newest first.
    let versions: Vec<u64> = store
        .list_versions(&s, "k")
        .unwrap()
        .iter()
        .map(|m| m.version)
        .collect();
    assert_eq!(versions, vec![2, 1]);
}

#[test]
fn a_pinned_version_still_resolves_after_a_later_put() {
    let (_d, store) = test_store();
    let s = scope();
    store.put(&s, "k", b"first", &owner()).unwrap();
    store.put(&s, "k", b"second", &owner()).unwrap();
    assert_eq!(store.get(&s, "k", Some(1)).unwrap(), b"first");
    assert_eq!(store.get(&s, "k", Some(2)).unwrap(), b"second");
}

#[test]
fn rotate_appends_and_stamps_rotated_at() {
    let (_d, store) = test_store();
    let s = scope();
    store.put(&s, "k", b"old", &owner()).unwrap();
    let (_, v) = store.rotate(&s, "k", b"new", &owner()).unwrap();
    assert_eq!(v, 2);
    assert_eq!(store.get(&s, "k", None).unwrap(), b"new");
    assert_eq!(store.get(&s, "k", Some(1)).unwrap(), b"old");

    let versions = store.list_versions(&s, "k").unwrap();
    assert!(versions[0].rotated_at.is_some(), "rotation is stamped");
    assert!(versions[1].rotated_at.is_none(), "a plain put is not");
}

// ---- the two distinct absences -----------------------------------------

#[test]
fn delete_makes_get_gone_while_a_pinned_version_still_works() {
    let (_d, store) = test_store();
    let s = scope();
    store.put(&s, "k", b"first", &owner()).unwrap();
    store.delete(&s, "k", &owner()).unwrap();

    assert!(
        matches!(store.get(&s, "k", None), Err(SecretError::Gone { .. })),
        "a deleted secret is Gone, not Pending"
    );
    assert_eq!(
        store.get(&s, "k", Some(1)).unwrap(),
        b"first",
        "prior versions survive a delete (MVCC time-travel reads need them)"
    );
}

#[test]
fn an_absent_version_is_pending_not_gone() {
    let (_d, store) = test_store();
    let s = scope();
    store.put(&s, "k", b"first", &owner()).unwrap();

    assert!(matches!(
        store.get(&s, "k", Some(9)),
        Err(SecretError::Pending {
            version: Some(9),
            ..
        })
    ));
    assert!(matches!(
        store.get(&s, "never-written", None),
        Err(SecretError::Pending { version: None, .. })
    ));
}

#[test]
fn a_tombstone_carries_no_ciphertext() {
    let (_d, store) = test_store();
    let s = scope();
    store.put(&s, "k", b"first", &owner()).unwrap();
    store.delete(&s, "k", &owner()).unwrap();
    let newest = &store.list_versions(&s, "k").unwrap()[0];
    assert!(newest.deleted);
    assert_eq!(newest.ciphertext_len, 0);
}

// ---- list leaks nothing -------------------------------------------------

#[test]
fn list_emits_metadata_only_never_ciphertext_or_plaintext() {
    let (_d, store) = test_store();
    let s = scope();
    let plaintext = b"sk_live_SUPERSECRET";
    store.put(&s, "stripe", plaintext, &owner()).unwrap();
    store.put(&s, "smtp", b"hunter2", &owner()).unwrap();

    let listed = store.list(&s).unwrap();
    assert_eq!(listed.len(), 2, "newest version of each name");

    // Serialize the whole listing and scan the bytes: this catches a leak
    // through any field, present or future, not just the ones named here.
    let blob = serde_json::to_vec(&listed).unwrap();
    assert!(
        !blob.windows(plaintext.len()).any(|w| w == plaintext),
        "plaintext must never appear in a listing"
    );
    assert!(
        !blob.windows(7).any(|w| w == b"hunter2"),
        "plaintext must never appear in a listing"
    );

    // And the ciphertext itself is absent too.
    let sealed = {
        let all = store.list_versions(&s, "stripe").unwrap();
        assert_eq!(all.len(), 1);
        all[0].ciphertext_len
    };
    assert!(sealed > 0, "there IS ciphertext at rest");
    let stripe = listed.iter().find(|m| m.name == "stripe").unwrap();
    assert_eq!(stripe.created_by, "alice");
    assert_eq!(stripe.version, 1);
}

#[test]
fn list_reports_the_newest_version_of_each_name() {
    let (_d, store) = test_store();
    let s = scope();
    store.put(&s, "k", b"a", &owner()).unwrap();
    store.put(&s, "k", b"b", &owner()).unwrap();
    store.put(&s, "k", b"c", &owner()).unwrap();
    store.put(&s, "other", b"x", &owner()).unwrap();

    let listed = store.list(&s).unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed.iter().find(|m| m.name == "k").unwrap().version, 3);
}

// ---- names --------------------------------------------------------------

#[test]
fn a_name_containing_nul_is_rejected() {
    let (_d, store) = test_store();
    let s = scope();
    assert!(matches!(
        store.put(&s, "bad\0name", b"x", &owner()),
        Err(SecretError::InvalidName(_))
    ));
    assert!(matches!(
        store.get(&s, "bad\0name", None),
        Err(SecretError::InvalidName(_))
    ));
    assert!(matches!(
        store.delete(&s, "bad\0name", &owner()),
        Err(SecretError::InvalidName(_))
    ));
}

#[test]
fn a_name_may_contain_slashes() {
    let (_d, store) = test_store();
    let s = scope();
    let name = raisin_models::secret_ref::node_field_secret_name("01H8XY", "venue.password");
    store.put(&s, &name, b"pw", &owner()).unwrap();
    assert_eq!(store.get(&s, &name, None).unwrap(), b"pw");
    assert_eq!(store.list(&s).unwrap()[0].name, name);
}

// ---- isolation ----------------------------------------------------------

#[test]
fn the_same_name_on_two_branches_is_two_secrets() {
    let (_d, store) = test_store();
    let main = SecretScope::new("acme", "site", "main");
    let feature = SecretScope::new("acme", "site", "feature");

    store.put(&main, "k", b"on-main", &owner()).unwrap();
    store.put(&feature, "k", b"on-feature", &owner()).unwrap();

    assert_eq!(store.get(&main, "k", None).unwrap(), b"on-main");
    assert_eq!(store.get(&feature, "k", None).unwrap(), b"on-feature");
    assert_eq!(store.list(&main).unwrap().len(), 1);
    assert_eq!(store.list(&feature).unwrap().len(), 1);

    // Deleting on one branch leaves the other alone.
    store.delete(&main, "k", &owner()).unwrap();
    assert!(matches!(
        store.get(&main, "k", None),
        Err(SecretError::Gone { .. })
    ));
    assert_eq!(store.get(&feature, "k", None).unwrap(), b"on-feature");
}

#[test]
fn a_different_tenant_or_repo_does_not_see_the_secret() {
    let (_d, store) = test_store();
    store.put(&scope(), "k", b"mine", &owner()).unwrap();

    for other in [
        SecretScope::new("other-tenant", "site", "main"),
        SecretScope::new("acme", "other-repo", "main"),
    ] {
        assert!(store.list(&other).unwrap().is_empty());
        assert!(matches!(
            store.get(&other, "k", None),
            Err(SecretError::Pending { .. })
        ));
    }
}

// ---- keyring ------------------------------------------------------------

#[test]
fn a_record_sealed_under_a_missing_key_id_is_refused_loudly() {
    let dir = TempDir::new().unwrap();
    enable_v2_emission();
    let mut opts = Options::default();
    opts.create_if_missing(true);
    opts.create_missing_column_families(true);
    let db = Arc::new(DB::open_cf(&opts, dir.path(), vec![cf::SECRETS]).unwrap());

    // Writer holds keys 1 and 2, seals under the active key 2.
    let writer = SecretStore::new(
        db.clone(),
        Arc::new(SecretBox::with_keyring(Arc::new(
            Keyring::new(vec![(1, [1u8; 32]), (2, [2u8; 32])], 2).unwrap(),
        ))),
        "writer",
    );
    writer.put(&scope(), "k", b"payload", &owner()).unwrap();
    assert_eq!(writer.list(&scope()).unwrap()[0].key_id, 2);

    // A peer holding only key 1 names the id it is missing, and its own.
    let peer = SecretStore::new(
        db,
        Arc::new(SecretBox::with_keyring(Arc::new(
            Keyring::new(vec![(1, [1u8; 32])], 1).unwrap(),
        ))),
        "peer",
    );
    let err = peer.get(&scope(), "k", None).unwrap_err();
    assert!(
        matches!(
            err,
            SecretError::UnknownKeyId {
                key_id: 2,
                ref available,
                ..
            } if available == &vec![1]
        ),
        "{err}"
    );
    let msg = err.to_string();
    assert!(msg.contains("master key id 2"), "{msg}");
}

#[test]
fn a_peer_with_the_wrong_key_material_fails_rather_than_returning_garbage() {
    let dir = TempDir::new().unwrap();
    enable_v2_emission();
    let mut opts = Options::default();
    opts.create_if_missing(true);
    opts.create_missing_column_families(true);
    let db = Arc::new(DB::open_cf(&opts, dir.path(), vec![cf::SECRETS]).unwrap());

    let writer = SecretStore::new(
        db.clone(),
        Arc::new(SecretBox::with_keyring(Arc::new(
            Keyring::new(vec![(1, [1u8; 32])], 1).unwrap(),
        ))),
        "writer",
    );
    writer.put(&scope(), "k", b"payload", &owner()).unwrap();

    // Same key id, different bytes: the id check passes and AEAD catches it.
    let impostor = SecretStore::new(
        db,
        Arc::new(SecretBox::with_keyring(Arc::new(
            Keyring::new(vec![(1, [9u8; 32])], 1).unwrap(),
        ))),
        "impostor",
    );
    assert!(matches!(
        impostor.get(&scope(), "k", None),
        Err(SecretError::Crypto(_))
    ));
}

// ---- storage layout -----------------------------------------------------

#[test]
fn stored_keys_sort_newest_first_under_a_forward_prefix_scan() {
    let (_d, store) = test_store();
    let s = scope();
    for i in 0..5u8 {
        store.put(&s, "k", &[i], &owner()).unwrap();
    }

    let handle = store.db.cf_handle(cf::SECRETS).unwrap();
    let prefix = super::keys::secret_versions_prefix(&s.tenant, &s.repo, &s.branch, "k");
    let mut seen = Vec::new();
    for item in store.db.iterator_cf(
        handle,
        rocksdb::IteratorMode::From(&prefix, rocksdb::Direction::Forward),
    ) {
        let (key, value) = item.unwrap();
        if !key.starts_with(&prefix) {
            break;
        }
        let r: SecretRecord = rmp_serde::from_slice(&value).unwrap();
        seen.push((super::keys::revision_from_key(&key).unwrap(), r.version));
    }

    assert_eq!(seen.len(), 5);
    assert_eq!(
        seen.iter().map(|(_, v)| *v).collect::<Vec<_>>(),
        vec![5, 4, 3, 2, 1],
        "a forward scan must be newest-first"
    );
    for w in seen.windows(2) {
        assert!(w[0].0 > w[1].0, "revisions descend");
    }
}

/// Two writes inside one millisecond must not collide. `HLC::now()` pins the
/// counter at 0 and would silently overwrite here; the ticking state does not.
#[test]
fn rapid_successive_writes_do_not_overwrite_each_other() {
    let (_d, store) = test_store();
    let s = scope();
    for i in 0..50u32 {
        store
            .put(&s, "k", i.to_string().as_bytes(), &owner())
            .unwrap();
    }
    let versions = store.list_versions(&s, "k").unwrap();
    assert_eq!(versions.len(), 50, "every write kept its own key");
    assert_eq!(store.get(&s, "k", None).unwrap(), b"49");
    assert_eq!(store.get(&s, "k", Some(1)).unwrap(), b"0");
}

#[test]
fn owner_node_and_field_are_recorded() {
    let (_d, store) = test_store();
    let s = scope();
    let o = SecretOwner::node_field("alice", "01H8XY", "venue.password");
    store
        .put(&s, "node/01H8XY/venue.password", b"pw", &o)
        .unwrap();
    let m = &store.list(&s).unwrap()[0];
    assert_eq!(m.owner_node.as_deref(), Some("01H8XY"));
    assert_eq!(m.owner_field.as_deref(), Some("venue.password"));
}

/// The v1 refusal, exercised without touching process-wide environment.
///
/// If this ever stops refusing, the store will happily write blobs that its own
/// `V1Policy::Reject` read path can never open — a failure that would only
/// surface at reveal time, looking like corruption.
#[test]
fn a_v1_envelope_is_refused_rather_than_stored() {
    let v1 = raisin_crypto::envelope::encode_v1(&[0u8; 12], &[0u8; 16]);
    assert!(matches!(
        super::crud::key_id_or_refuse(&v1),
        Err(SecretError::V2EmissionRequired)
    ));
    let v2 = raisin_crypto::envelope::encode_v2(7, &[0u8; 12], &[0u8; 16]);
    assert_eq!(super::crud::key_id_or_refuse(&v2).unwrap(), 7);
}

#[test]
fn a_missing_actor_is_stamped_anonymous() {
    let (_d, store) = test_store();
    let s = scope();
    store.put(&s, "k", b"x", &SecretOwner::default()).unwrap();
    assert_eq!(store.list(&s).unwrap()[0].created_by, "anonymous");
}
