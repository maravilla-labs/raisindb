//! Plan Phase 4: the batched read equals `get` for random ids and revisions.
//!
//! A seeded random history drives the real write funnels — repository create,
//! transaction `put_node` (create, rename, property update), a legacy
//! full-`Node` record (what a pre-Phase-10 binary left), repository delete and
//! `move_node_tree` — recording HEAD after every step. Then at EVERY recorded
//! revision a shuffled list of items (every id and path the history ever used,
//! duplicates, a missing id) is read both ways and compared, under every
//! `ReadOpts` (`assert_equals_get`). The MVCC oracle in raisin-sql-execution
//! asks the same question against its independent model (`batch_get`
//! template).

use crate::batch_get_test::{assert_equals_get, branch, id, path};
use crate::node_path_writer_test::{
    folder, head, legacy_tx_put, repo_create, setup, tx_put, BRANCH, REPO, TENANT, WS,
};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::NodeType;
use raisin_storage::{
    BatchReadItem, CommitMetadata, DeleteNodeOptions, NodeRepository, NodeTypeRepository, Storage,
    StorageScope,
};
use std::collections::{BTreeMap, BTreeSet};

/// A tiny deterministic generator (xorshift64*): the history is a pure
/// function of the seed, so a failure names a reproducible case.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// Live nodes as the history sees them: id -> path.
type Live = BTreeMap<String, String>;

fn leaves(live: &Live) -> Vec<String> {
    live.iter()
        .filter(|(_, p)| !live.values().any(|q| q.starts_with(&format!("{p}/"))))
        .map(|(id, _)| id.clone())
        .collect()
}

fn pick(rng: &mut Rng, from: &[String]) -> Option<String> {
    (!from.is_empty()).then(|| from[rng.below(from.len())].clone())
}

/// One random step. Returns false when the step had nothing to act on.
async fn step(
    storage: &raisin_rocksdb::RocksDBStorage,
    rng: &mut Rng,
    live: &mut Live,
    next_id: &mut usize,
) -> Result<bool> {
    let scope = StorageScope::new(TENANT, REPO, BRANCH, WS);
    let ids: Vec<String> = live.keys().cloned().collect();
    match rng.below(7) {
        // Create, at the root or under a live node, through either layer.
        0 | 1 => {
            let id = format!("n{next_id}");
            *next_id += 1;
            let parent = if rng.below(2) == 0 {
                None
            } else {
                pick(rng, &ids).map(|p| live[&p].clone())
            };
            let path = format!("{}/{id}", parent.as_deref().unwrap_or(""));
            if parent.is_none() && rng.below(2) == 0 {
                repo_create(storage, folder(&id, &path)).await?;
            } else {
                tx_put(storage, &folder(&id, &path)).await?;
            }
            live.insert(id, path);
        }
        // Rename a leaf in place (same parent) through put_node — sometimes
        // as the legacy record a pre-Phase-10 binary wrote.
        2 => {
            let Some(id) = pick(rng, &leaves(live)) else {
                return Ok(false);
            };
            let old = live[&id].clone();
            let parent = &old[..old.rfind('/').unwrap()];
            let path = format!("{parent}/{id}r{}", rng.below(1000));
            if rng.below(2) == 0 {
                legacy_tx_put(storage, &folder(&id, &path)).await?;
            } else {
                tx_put(storage, &folder(&id, &path)).await?;
            }
            live.insert(id, path);
        }
        // A property update at the same path.
        3 => {
            let Some(id) = pick(rng, &ids) else {
                return Ok(false);
            };
            let mut node = folder(&id, &live[&id]);
            node.properties.insert(
                "v".into(),
                PropertyValue::String(format!("{}", rng.next() % 97)),
            );
            tx_put(storage, &node).await?;
        }
        // Delete a leaf.
        4 => {
            let Some(id) = pick(rng, &leaves(live)) else {
                return Ok(false);
            };
            storage
                .nodes()
                .delete(scope, &id, DeleteNodeOptions::default())
                .await?;
            live.remove(&id);
        }
        // Move a subtree to the root under a fresh name.
        _ => {
            let Some(id) = pick(rng, &ids) else {
                return Ok(false);
            };
            let old = live[&id].clone();
            let new = format!("/m{}", *next_id);
            *next_id += 1;
            storage
                .nodes()
                .move_node_tree(scope, &id, &new, None)
                .await?;
            for p in live.values_mut() {
                if *p == old || p.starts_with(&format!("{old}/")) {
                    *p = format!("{new}{}", &p[old.len()..]);
                }
            }
        }
    }
    Ok(true)
}

/// `raisin:Folder`, allowing any child: a nested `put_node` validates its
/// parent's type.
async fn seed_folder_type(storage: &raisin_rocksdb::RocksDBStorage) -> Result<()> {
    let mut folder_type: NodeType =
        serde_json::from_value(serde_json::json!({ "name": "raisin:Folder" }))
            .expect("node type literal");
    folder_type.id = Some("raisin:Folder".to_string());
    folder_type.strict = Some(false);
    folder_type.allowed_children = vec!["*".to_string()];
    storage
        .node_types()
        .upsert(branch(), folder_type, CommitMetadata::system("seed"))
        .await?;
    Ok(())
}

#[tokio::test]
async fn batch_get_equals_get_for_random_ids_and_revisions() -> Result<()> {
    let seeds: u64 = std::env::var("BATCH_GET_SEEDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4);
    for seed in 1..=seeds {
        let (storage, _dir) = setup().await?;
        seed_folder_type(&storage).await?;
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ seed.wrapping_mul(0xD1B5_4A32_D192_ED03));
        let mut live = Live::new();
        let mut next_id = 0;
        let mut revisions: Vec<HLC> = Vec::new();
        let mut ids_seen: BTreeSet<String> = BTreeSet::new();
        let mut paths_seen: BTreeSet<String> = BTreeSet::new();
        let mut steps = 0;
        while steps < 40 {
            if step(&storage, &mut rng, &mut live, &mut next_id).await? {
                steps += 1;
                revisions.push(head(&storage).await?);
                ids_seen.extend(live.keys().cloned());
                paths_seen.extend(live.values().cloned());
            }
        }

        let mut items: Vec<BatchReadItem> = ids_seen.iter().map(|i| id(i)).collect();
        items.extend(paths_seen.iter().map(|p| path(p)));
        items.push(id("never-created"));
        items.push(path("/never/there"));
        for _ in 0..4 {
            let dup = items[rng.below(items.len())].clone();
            items.push(dup);
        }
        // Shuffle: the batch sorts internally and must answer in input order.
        for i in (1..items.len()).rev() {
            items.swap(i, rng.below(i + 1));
        }

        revisions.dedup();
        for rev in &revisions {
            assert_equals_get(&storage, &items, rev, &format!("seed {seed} at {rev}")).await;
        }
        // And every node the history ended with is found, at its path.
        let at_head = assert_equals_get(&storage, &items, revisions.last().unwrap(), "HEAD").await;
        for (item, node) in items.iter().zip(&at_head) {
            if let raisin_storage::NodeLocator::Id(i) = &item.locator {
                assert_eq!(
                    node.as_ref().map(|n| n.path.clone()),
                    live.get(i).cloned(),
                    "seed {seed}: {i} at HEAD"
                );
            }
        }
    }
    Ok(())
}
