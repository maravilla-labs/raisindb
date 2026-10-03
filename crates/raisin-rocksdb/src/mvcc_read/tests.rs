//! The seek must answer exactly what the walk it replaced answered.

use super::newest_at_or_before;
use raisin_hlc::HLC;
use rocksdb::{Options, DB};

const PREFIX: &[u8] = b"t\0r\0main\0ws\0nodes\0n1\0";

fn open() -> (DB, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let mut opts = Options::default();
    opts.create_if_missing(true);
    opts.create_missing_column_families(true);
    (DB::open_cf(&opts, dir.path(), [CF]).unwrap(), dir)
}

/// A named CF, not "default": `cf_handle("default")` returns `None` in this
/// rust-rocksdb mode.
const CF: &str = "versions";

fn put(db: &DB, key: impl AsRef<[u8]>, value: impl AsRef<[u8]>) {
    db.put_cf(db.cf_handle(CF).unwrap(), key, value).unwrap();
}

fn key(prefix: &[u8], rev: &HLC) -> Vec<u8> {
    let mut key = prefix.to_vec();
    key.extend_from_slice(&rev.encode_descending());
    key
}

/// The pre-seek reader: walk the prefix newest first, skip keys whose revision
/// does not parse, return the first revision `<= max`.
fn walk(db: &DB, prefix: &[u8], max: Option<&HLC>) -> Option<(HLC, Vec<u8>)> {
    let cf = db.cf_handle(CF).unwrap();
    for item in crate::prefix_scan(db, cf, prefix) {
        let (key, value) = item.unwrap();
        if !key.starts_with(prefix) {
            break;
        }
        let Ok(rev) = crate::keys::extract_revision_from_key(&key) else {
            continue;
        };
        if max.is_none_or(|max| &rev <= max) {
            return Some((rev, value.to_vec()));
        }
    }
    None
}

fn seek(db: &DB, prefix: &[u8], max: Option<&HLC>) -> Option<(HLC, Vec<u8>)> {
    let cf = db.cf_handle(CF).unwrap();
    newest_at_or_before(db, cf, prefix, max).unwrap()
}

/// Deterministic xorshift, so the test needs no `rand` and never flakes.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// Revisions whose descending encodings contain `0x00` and `0xFF` bytes: a
/// `0xFF` timestamp byte becomes `0x00`, a counter of 0 is eight `0xFF`s and a
/// counter of `u64::MAX` is eight `0x00`s.
fn awkward_revisions() -> Vec<HLC> {
    vec![
        HLC::new(0x0000_01FF_00FF_FF00, u64::MAX),
        HLC::new(0x0000_01FF_00FF_FF00, 0),
        HLC::new(0x0000_0100_FF00_00FF, 1),
        HLC::new(0x0000_0100_FF00_00FF, 0xFF00),
        HLC::new(1_705_843_009_213, 0),
        HLC::new(1_705_843_009_213, u64::MAX - 1),
        HLC::new(u64::MAX - 5, 0),
        HLC::new(u64::MAX - 5, u64::MAX - 1),
        HLC::new(1, 0),
    ]
}

#[test]
fn seek_matches_walk_over_random_and_awkward_revisions() {
    let (db, _dir) = open();
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);

    let mut revisions = awkward_revisions();
    for _ in 0..300 {
        // Mix narrow and wide timestamps so the encodings vary in every byte.
        let ts = if rng.next() % 2 == 0 {
            1_700_000_000_000 + rng.next() % 1_000_000
        } else {
            rng.next()
        };
        revisions.push(HLC::new(ts, rng.next() % 4));
    }
    for (i, rev) in revisions.iter().enumerate() {
        put(&db, key(PREFIX, rev), format!("v{i}"));
    }

    // Neighbours that must never be read: a longer node id sharing the
    // textual prefix, and the next prefix up.
    let newest = HLC::new(u64::MAX, u64::MAX);
    put(&db, key(b"t\0r\0main\0ws\0nodes\0n10\0", &newest), "other");
    put(&db, key(b"t\0r\0main\0ws\0nodes\0n1\x01", &newest), "other");

    let mut targets = revisions.clone();
    targets.extend(
        awkward_revisions()
            .iter()
            .map(|r| HLC::new(r.timestamp_ms, r.counter.wrapping_add(1))),
    );
    targets.push(HLC::new(0, 0));
    targets.push(HLC::new(u64::MAX, u64::MAX));
    for _ in 0..300 {
        targets.push(HLC::new(rng.next(), rng.next()));
    }

    for target in &targets {
        assert_eq!(
            seek(&db, PREFIX, Some(target)),
            walk(&db, PREFIX, Some(target)),
            "seek and walk disagree at {target}"
        );
    }
    assert_eq!(seek(&db, PREFIX, None), walk(&db, PREFIX, None));
}

/// A key whose revision cannot be decoded is skipped, never the end of the
/// read — including when the seek lands directly on it.
#[test]
fn seek_advances_past_an_unparseable_key() {
    let (db, _dir) = open();
    let prefix: &[u8] = b"p\0";

    let answer = HLC::new(u64::MAX - 5, 0);
    let target = HLC::new(u64::MAX - 5, u64::MAX - 1);
    put(&db, key(prefix, &answer), "answer");

    // 12 bytes long, so `extract_revision_from_key` fails; it sorts strictly
    // between the seek position and the answer.
    let broken = [b'p', 0, 0, 0, 0, 0, 0, 0, 0, 5, 0, 1];
    put(&db, broken, "broken");
    let seek_position = key(prefix, &target);
    assert!(seek_position.as_slice() < broken.as_slice());
    assert!(broken.as_slice() < key(prefix, &answer).as_slice());

    let found = seek(&db, prefix, Some(&target));
    assert_eq!(found, Some((answer, b"answer".to_vec())));
    assert_eq!(found, walk(&db, prefix, Some(&target)));
}

#[test]
fn nothing_at_or_before_the_target_is_none() {
    let (db, _dir) = open();
    put(&db, key(PREFIX, &HLC::new(100, 0)), "v");

    assert_eq!(seek(&db, PREFIX, Some(&HLC::new(99, u64::MAX))), None);
    assert_eq!(
        seek(&db, PREFIX, Some(&HLC::new(100, 0))),
        Some((HLC::new(100, 0), b"v".to_vec()))
    );
    assert_eq!(seek(&db, b"t\0r\0main\0ws\0nodes\0absent\0", None), None);
}
