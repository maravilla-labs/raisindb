//! Weighted Reciprocal Rank Fusion over N legs, and the total order the results
//! come back in.
//!
//! # Why N legs and not two
//!
//! There were two: one full-text, one vector. A second vector leg is not
//! hypothetical -- `cf::EMBEDDINGS` has always carried a `kind` segment
//! (`T`/`I`) and `PartitionId` renders it, so an image tower is one partition
//! away. Written as a two-leg function, `fuse(fulltext, vector, w_ft, w_vec)`,
//! adding it means a third parameter, a third rank map and a third branch at
//! every call site -- and this codebase's named #1 bug class is exactly the
//! mirrored path that drifts. A leg is therefore a VALUE in a list. Adding the
//! image tower adds no parameter and no branch here.
//!
//! # Fusion is rank-based, and that is enforced by the types
//!
//! A leg contributes [`LegResult::ordered`]: its hits, best first. The rank is
//! the POSITION in that vector, computed here. There is deliberately no
//! `score`, `distance` or `similarity` field anywhere in the fusion input.
//!
//! That absence is the point, and it is structural rather than a comment,
//! because the failure it prevents is silent. Two vector partitions are two
//! DIFFERENT embedding spaces: a cosine distance of 0.31 from a text tower and
//! one of 0.31 from an image tower are not the same quantity and are not
//! comparable, any more than 0.31 metres is 0.31 seconds. Every arithmetic
//! combination of them is finite, every resulting ranking is plausible, and
//! nothing anywhere logs a fault -- the same shape as the two-embedders-one-index
//! bug that `PartitionId` exists to prevent. A maintainer reaching for
//! `score += weight * (1.0 - distance)` here finds no distance to reach for.
//!
//! Distances still reach the CALLER, as the `vector_distance` column: reporting
//! a measurement is fine, fusing two incommensurable ones is not. They travel
//! beside the ranks in [`VectorDetails`], keyed by leg, and are attached after
//! scoring.

use std::collections::HashMap;

use super::RRF_K;

/// A hit is identified by `(workspace_id, node_id)`, never by `node_id` alone.
///
/// A node id is unique only within its workspace, and the workspace is the half
/// needed to fetch the node back and to permission-check it in the right scope.
/// Both index legs report it, so it is carried through fusion rather than
/// re-guessed at fetch time.
pub type HitKey = (String, String);

/// Which leg a rank came from.
///
/// Ordered so that `Fulltext` sorts before any vector leg and vector legs sort
/// by partition, giving [`FusedHit::ranks`] a stable presentation order.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LegId {
    /// The lexical leg. There is at most one.
    Fulltext,
    /// One vector partition, i.e. one embedding space.
    ///
    /// `kind` is the partition's kind character (`'T'` text, `'I'` image) and
    /// `partition` its full token. Both are carried because the kind is what the
    /// caller asked for and is reported as a column, while the token is what
    /// distinguishes two partitions of the SAME kind -- which is what happens
    /// for the length of a re-embedding, when the old and the new model's
    /// indexes both exist.
    Vector { kind: char, partition: String },
}

impl LegId {
    pub fn is_vector(&self) -> bool {
        matches!(self, LegId::Vector { .. })
    }

    /// The partition kind character, for a vector leg.
    pub fn kind_char(&self) -> Option<char> {
        match self {
            LegId::Vector { kind, .. } => Some(*kind),
            LegId::Fulltext => None,
        }
    }

    /// A short label for `EXPLAIN` and the operator log.
    pub fn label(&self) -> String {
        match self {
            LegId::Fulltext => "fulltext".to_string(),
            LegId::Vector { kind, partition } => format!("vector:{kind}:{partition}"),
        }
    }
}

/// One leg's contribution to fusion.
///
/// `ordered` is the leg's hits, BEST FIRST. The rank fused on is the position in
/// that vector; see the module docs for why no score may live here.
#[derive(Debug, Clone)]
pub struct LegResult {
    pub leg: LegId,
    /// This leg's weight in the fused score. Every vector leg carries the
    /// caller's `vector_weight` in full rather than a share of it, so a document
    /// found by two towers scores above one found by either alone -- which is
    /// the entire point of asking for both.
    pub weight: f64,
    /// Hits in rank order, best first.
    pub ordered: Vec<HitKey>,
    /// What this leg was ASKED for. `ordered.len() < requested` is the only
    /// available evidence that the leg has nothing more to give.
    pub requested: usize,
}

impl LegResult {
    /// Did this leg come back short of what it was asked for?
    pub fn exhausted(&self) -> bool {
        self.ordered.len() < self.requested
    }
}

/// Per-hit detail from a vector leg: a measurement, not a score.
///
/// Kept OUT of [`LegResult`] so that fusion cannot read it. See the module docs.
#[derive(Debug, Clone, Copy)]
pub struct VectorDetail {
    pub distance: f32,
    pub chunk_index: usize,
}

/// Vector detail keyed by `(leg, hit)`.
///
/// Keyed by leg as well as hit because the same node can be returned by two
/// partitions with two unrelated distances, and collapsing them would silently
/// report one space's measurement under the other's.
pub type VectorDetails = HashMap<(LegId, HitKey), VectorDetail>;

/// One fused candidate.
#[derive(Debug, Clone)]
pub struct FusedHit {
    pub key: HitKey,
    pub score: f64,
    /// Every leg that returned this hit, with its 1-based rank, ordered by leg.
    pub ranks: Vec<(LegId, usize)>,
    /// The distance reported by the best-RANKED vector leg, if any.
    pub vector_distance: Option<f32>,
    /// The chunk matched by the best-RANKED vector leg, if any.
    pub chunk_index: Option<usize>,
    /// The kind character of the best-RANKED vector leg, if any.
    pub embedding_kind: Option<char>,
}

impl FusedHit {
    /// The lexical rank, or `None` when the full-text leg did not return this
    /// hit -- which includes the leg not having run at all.
    pub fn fulltext_rank(&self) -> Option<usize> {
        self.ranks
            .iter()
            .find(|(leg, _)| *leg == LegId::Fulltext)
            .map(|(_, rank)| *rank)
    }

    /// The best rank across every vector leg.
    ///
    /// BEST RANK, not smallest distance. Ranks are ordinals within one leg and
    /// so are comparable across legs in a way the distances behind them are
    /// not: "first out of this tower" means the same thing whichever tower said
    /// it. Picking by distance instead would compare two embedding spaces'
    /// measurements and prefer whichever space happens to produce smaller
    /// numbers -- always the same one, for every query, silently.
    pub fn vector_rank(&self) -> Option<usize> {
        self.ranks
            .iter()
            .filter(|(leg, _)| leg.is_vector())
            .map(|(_, rank)| *rank)
            .min()
    }
}

/// `score = Σ_legs weight_leg / (RRF_K + rank_leg)`.
///
/// With one full-text and one vector leg at the default `1.0` weights this is
/// numerically identical to the previous two-leg arithmetic, not merely
/// order-preserving -- which is why two independent weights beat a single
/// `semantic_weight` whose 0.5 default would halve every published score.
///
/// Every leg's ranks MUST come from the same run. When the emit loop re-draws at
/// a larger `k` it re-runs every leg and calls this again from scratch: HNSW is
/// approximate, so a wider search can reorder, and ranks stitched across two
/// runs would make `vector_rank` a lie in a column that says otherwise.
pub fn fuse(legs: &[LegResult], details: &VectorDetails) -> Vec<FusedHit> {
    // rank maps, one per leg, built here so that a rank is by construction a
    // position in `ordered` and cannot be supplied by a caller.
    let ranked: Vec<(&LegResult, HashMap<&HitKey, usize>)> = legs
        .iter()
        .map(|leg| {
            let map = leg
                .ordered
                .iter()
                .enumerate()
                .map(|(i, key)| (key, i + 1))
                .collect();
            (leg, map)
        })
        .collect();

    let mut keys: Vec<HitKey> = legs
        .iter()
        .flat_map(|leg| leg.ordered.iter().cloned())
        .collect();
    keys.sort();
    keys.dedup();

    let mut hits: Vec<FusedHit> = keys
        .into_iter()
        .map(|key| {
            let mut score = 0.0;
            let mut ranks: Vec<(LegId, usize)> = Vec::new();
            for (leg, map) in &ranked {
                if let Some(rank) = map.get(&key).copied() {
                    score += leg.weight / (RRF_K + rank as f64);
                    ranks.push((leg.leg.clone(), rank));
                }
            }
            ranks.sort();

            // Detail comes from the best-ranked VECTOR leg. See `vector_rank`.
            let best_vector = ranks
                .iter()
                .filter(|(leg, _)| leg.is_vector())
                .min_by_key(|(_, rank)| *rank);
            let (vector_distance, chunk_index, embedding_kind) = match best_vector {
                Some((leg, _)) => {
                    let detail = details.get(&(leg.clone(), key.clone()));
                    (
                        detail.map(|d| d.distance),
                        detail.map(|d| d.chunk_index),
                        leg.kind_char(),
                    )
                }
                None => (None, None, None),
            };

            FusedHit {
                key,
                score,
                ranks,
                vector_distance,
                chunk_index,
                embedding_kind,
            }
        })
        .collect();

    sort_total(&mut hits);
    hits
}

/// `(score DESC, workspace_id ASC, node_id ASC)`.
///
/// A TOTAL order, not `partial_cmp(...).unwrap_or(Equal)`. RRF produces exact
/// ties routinely (any two hits at the same rank in the same single leg tie), and
/// a caching agent that re-runs a query needs the same rows in the same order.
pub fn sort_total(hits: &mut [FusedHit]) {
    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.key.0.cmp(&b.key.0))
            .then_with(|| a.key.1.cmp(&b.key.1))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(ws: &str, node: &str) -> HitKey {
        (ws.to_string(), node.to_string())
    }

    fn text_leg() -> LegId {
        LegId::Vector {
            kind: 'T',
            partition: "txt".to_string(),
        }
    }

    fn image_leg() -> LegId {
        LegId::Vector {
            kind: 'I',
            partition: "img".to_string(),
        }
    }

    fn leg(id: LegId, weight: f64, hits: &[HitKey]) -> LegResult {
        LegResult {
            leg: id,
            weight,
            ordered: hits.to_vec(),
            requested: 10,
        }
    }

    fn detail(details: &mut VectorDetails, leg: LegId, k: HitKey, distance: f32, chunk: usize) {
        details.insert(
            (leg, k),
            VectorDetail {
                distance,
                chunk_index: chunk,
            },
        );
    }

    /// The two-leg default must stay numerically identical to what shipped, or
    /// every published score changes under a refactor that promised not to.
    #[test]
    fn default_weights_reproduce_plain_rrf() {
        let a = key("w", "a");
        let mut details = VectorDetails::new();
        detail(&mut details, text_leg(), a.clone(), 0.1, 0);

        let legs = vec![
            leg(LegId::Fulltext, 1.0, &[a.clone()]),
            leg(text_leg(), 1.0, &[key("w", "other"), a.clone()]),
        ];
        let hits = fuse(&legs, &details);
        let hit = hits.iter().find(|h| h.key == a).unwrap();
        assert!((hit.score - (1.0 / 61.0 + 1.0 / 62.0)).abs() < 1e-12);
        assert_eq!(hit.fulltext_rank(), Some(1));
        assert_eq!(hit.vector_rank(), Some(2));
    }

    #[test]
    fn a_zero_weight_contributes_nothing() {
        let a = key("w", "a");
        let legs = vec![leg(LegId::Fulltext, 0.0, &[a.clone()])];
        let hits = fuse(&legs, &VectorDetails::new());
        assert_eq!(hits[0].score, 0.0);
        assert_eq!(hits[0].vector_rank(), None);
    }

    /// Ties must break the same way every run, or a caching agent sees the rows
    /// shuffle under it.
    #[test]
    fn ties_break_on_workspace_then_node() {
        let legs = vec![leg(
            LegId::Fulltext,
            1.0,
            // All at rank 1 is impossible within one leg, so give them the same
            // score by making each leg one element long.
            &[key("b", "2")],
        )];
        let mut all = Vec::new();
        for k in [key("b", "2"), key("a", "2"), key("a", "1")] {
            all.push(leg(
                LegId::Vector {
                    kind: 'T',
                    partition: format!("p{}{}", k.0, k.1),
                },
                1.0,
                &[k],
            ));
        }
        let _ = legs;
        let hits = fuse(&all, &VectorDetails::new());
        let order: Vec<_> = hits.iter().map(|h| h.key.clone()).collect();
        assert_eq!(order, vec![key("a", "1"), key("a", "2"), key("b", "2")]);
    }

    /// A THIRD leg joins with no signature change and no new branch. This is
    /// the property the generalisation exists for.
    #[test]
    fn a_third_leg_fuses_without_any_new_parameter() {
        let shared = key("w", "shared");
        let text_only = key("w", "text-only");
        let image_only = key("w", "image-only");

        let legs = vec![
            leg(LegId::Fulltext, 1.0, &[text_only.clone()]),
            leg(text_leg(), 1.0, &[shared.clone()]),
            leg(image_leg(), 1.0, &[shared.clone(), image_only.clone()]),
        ];
        let hits = fuse(&legs, &VectorDetails::new());

        // Found by two legs, so it outscores anything found by one.
        assert_eq!(hits[0].key, shared);
        assert_eq!(hits[0].ranks.len(), 2);
        assert!((hits[0].score - 2.0 / 61.0).abs() < 1e-12);
        assert_eq!(hits.len(), 3);
    }

    /// The distance reported is the one belonging to the best-RANKED leg, not
    /// the numerically smallest across two incommensurable spaces.
    #[test]
    fn detail_follows_rank_not_the_smaller_distance() {
        let shared = key("w", "shared");
        let mut details = VectorDetails::new();
        // The image tower ranks it FIRST but reports a larger number; the text
        // tower ranks it second with a smaller one. Rank wins.
        detail(&mut details, image_leg(), shared.clone(), 0.42, 7);
        detail(&mut details, text_leg(), shared.clone(), 0.05, 3);

        let legs = vec![
            leg(image_leg(), 1.0, &[shared.clone()]),
            leg(text_leg(), 1.0, &[key("w", "filler"), shared.clone()]),
        ];
        let hits = fuse(&legs, &details);
        let hit = hits.iter().find(|h| h.key == shared).unwrap();
        assert_eq!(hit.vector_rank(), Some(1));
        assert_eq!(hit.vector_distance, Some(0.42));
        assert_eq!(hit.chunk_index, Some(7));
        assert_eq!(hit.embedding_kind, Some('I'));
    }

    /// Two partitions of the same kind (an in-flight re-embedding) keep their
    /// own detail rather than collapsing onto one another.
    #[test]
    fn two_partitions_of_one_kind_do_not_share_detail() {
        let k = key("w", "n");
        let old = LegId::Vector {
            kind: 'T',
            partition: "old".to_string(),
        };
        let new = LegId::Vector {
            kind: 'T',
            partition: "new".to_string(),
        };
        let mut details = VectorDetails::new();
        detail(&mut details, old.clone(), k.clone(), 0.9, 1);
        detail(&mut details, new.clone(), k.clone(), 0.1, 2);

        let legs = vec![
            leg(new.clone(), 1.0, &[k.clone()]),
            leg(old.clone(), 1.0, &[key("w", "filler"), k.clone()]),
        ];
        let hits = fuse(&legs, &details);
        assert_eq!(hits[0].vector_distance, Some(0.1));
        assert_eq!(hits[0].chunk_index, Some(2));
    }

    #[test]
    fn higher_score_first() {
        let legs = vec![leg(
            LegId::Fulltext,
            1.0,
            &[key("w", "first"), key("w", "second")],
        )];
        let hits = fuse(&legs, &VectorDetails::new());
        assert_eq!(hits[0].key.1, "first");
    }

    #[test]
    fn a_short_leg_reports_itself_exhausted() {
        let full = LegResult {
            leg: LegId::Fulltext,
            weight: 1.0,
            ordered: vec![key("w", "a"), key("w", "b")],
            requested: 2,
        };
        assert!(!full.exhausted());
        let short = LegResult {
            requested: 3,
            ..full
        };
        assert!(short.exhausted());
    }
}
