//! Subtree document order: the `__tree_order` encoding.
//!
//! A node's `tree_order` is the chain of its ancestors' order labels, root-first,
//! joined by [`TREE_ORDER_SEPARATOR`]. Sorting these strings byte-wise reproduces
//! a pre-order depth-first walk of the subtree, which makes a single `Text` value
//! both the sort key and the keyset cursor for paging an entire subtree.
//!
//! # Why the ordering works
//!
//! Two properties combine:
//!
//! 1. **An ancestor's value is a byte prefix of its descendants' values**, so it
//!    always sorts first — a node precedes its own subtree.
//! 2. **Two sibling subtrees diverge inside their own labels**, before any
//!    separator is reached, because order labels are prefix-free (see
//!    [`crate::fractional_index::SEPARATOR`]). So sibling subtrees stay
//!    contiguous and in editorial order.
//!
//! The separator is a space (`0x20`): the lowest printable byte, below every byte
//! a label can contain (hex digits `0x30..=0x66` and `':'` at `0x3A`). Being
//! printable, it survives the pgwire text protocol and JSON without escaping —
//! unlike a control byte or NUL.
//!
//! Values are **not stored**. A reorder would invalidate every descendant's
//! value, so they are computed during traversal instead, where the ancestor
//! labels are already in hand.
//!
//! Note the deliberate distinction from a node's `path` (`/menu/a`): both order
//! parents before children, but `path` orders siblings alphabetically while
//! `tree_order` orders them editorially. They only agree when the editorial order
//! happens to be alphabetical.

/// Separator between levels of a `tree_order` value.
///
/// See the module docs for why a space is the right choice.
pub(in crate::repositories::nodes) const TREE_ORDER_SEPARATOR: char = ' ';

/// Append one level's order label to a parent's `tree_order`.
///
/// The traversal root has an empty value, so its children's values are bare
/// labels with no leading separator.
pub(in crate::repositories::nodes) fn join_tree_order(parent: &str, label: &str) -> String {
    if parent.is_empty() {
        label.to_string()
    } else {
        format!("{parent}{TREE_ORDER_SEPARATOR}{label}")
    }
}

/// Split an `tree_order` back into its per-level labels.
///
/// The inverse of repeated [`join_tree_order`]. An empty value yields no labels,
/// which denotes the traversal root itself.
pub(in crate::repositories::nodes) fn split_tree_order(tree_order: &str) -> Vec<&str> {
    if tree_order.is_empty() {
        Vec::new()
    } else {
        tree_order.split(TREE_ORDER_SEPARATOR).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fractional_index;
    use raisin_hlc::HLC;

    fn label(seed: u64) -> String {
        let mut fractional = fractional_index::first();
        for _ in 0..seed {
            fractional = fractional_index::inc(&fractional).expect("inc");
        }
        fractional_index::format_label(&fractional, &HLC::new(1_700_000_000_000 + seed, 0))
    }

    #[test]
    fn join_and_split_round_trip() {
        let (a, b, c) = (label(0), label(1), label(2));

        let level1 = join_tree_order("", &a);
        assert_eq!(level1, a, "the first level carries no leading separator");

        let level2 = join_tree_order(&level1, &b);
        let level3 = join_tree_order(&level2, &c);

        assert_eq!(
            split_tree_order(&level3),
            vec![a.as_str(), b.as_str(), c.as_str()]
        );
        assert_eq!(split_tree_order(&level1), vec![a.as_str()]);
        assert!(split_tree_order("").is_empty());
    }

    /// The separator must sort below every byte an order label can contain, or a
    /// deeper path could sort before a shallower sibling.
    #[test]
    fn separator_sorts_below_every_label_byte() {
        let separator = TREE_ORDER_SEPARATOR as u8;
        for seed in 0..24 {
            for byte in label(seed).bytes() {
                assert!(
                    separator < byte,
                    "separator {separator:#04x} must sort below label byte {byte:#04x} ({:?})",
                    byte as char
                );
            }
        }
    }

    /// The load-bearing property: sorting order paths byte-wise reproduces
    /// pre-order depth-first traversal.
    #[test]
    fn sorting_tree_orders_yields_document_order() {
        let (a, b, c) = (label(0), label(1), label(2));

        // Tree:  A ── A1 ── A1a
        //          └─ A2
        //        B
        let root_a = join_tree_order("", &a);
        let a1 = join_tree_order(&root_a, &a);
        let a1a = join_tree_order(&a1, &a);
        let a2 = join_tree_order(&root_a, &b);
        let root_b = join_tree_order("", &b);
        let root_c = join_tree_order("", &c);

        let document_order = vec![
            root_a.clone(),
            a1.clone(),
            a1a.clone(),
            a2.clone(),
            root_b.clone(),
            root_c.clone(),
        ];

        let mut shuffled = vec![a2, root_c, a1a, root_a, root_b, a1];
        shuffled.sort();

        assert_eq!(
            shuffled, document_order,
            "byte-wise sort must reproduce pre-order DFS"
        );
    }

    /// An ancestor must sort before its whole subtree, and a subtree must stay
    /// contiguous — never interleaved with a sibling's.
    #[test]
    fn subtrees_are_contiguous_and_follow_their_ancestor() {
        let (a, b) = (label(0), label(1));

        let first = join_tree_order("", &a);
        let first_child = join_tree_order(&first, &a);
        let first_deep = join_tree_order(&first_child, &b);
        let second = join_tree_order("", &b);

        assert!(first < first_child, "ancestor precedes its child");
        assert!(first_child < first_deep, "child precedes its own child");
        assert!(
            first_deep < second,
            "the whole first subtree precedes the next sibling"
        );
    }
}
