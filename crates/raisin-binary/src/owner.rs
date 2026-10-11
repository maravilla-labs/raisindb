//! Which process an uncommitted write belongs to.
//!
//! A boot sweep that removes partial writes by AGE alone is wrong twice over:
//! on a filesystem or bucket shared by several nodes it removes another
//! node's live write, and a long, healthy write (a multi-hour package build)
//! outlives any fixed cutoff. So every partial carries its owner — the node
//! id and a per-process boot id — and a sweep removes only partials of THIS
//! node from a PREVIOUS boot: their process is gone, so they are provably
//! dead. Another node's partials, and this process's own, are never touched.

use std::sync::OnceLock;

/// The writer identity stamped on every partial.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Owner {
    pub node: String,
    pub boot: String,
}

const ALNUM: [char; 62] = [
    '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i',
    'j', 'k', 'l', 'm', 'n', 'o', 'p', 'q', 'r', 's', 't', 'u', 'v', 'w', 'x', 'y', 'z', 'A', 'B',
    'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L', 'M', 'N', 'O', 'P', 'Q', 'R', 'S', 'T', 'U',
    'V', 'W', 'X', 'Y', 'Z',
];

/// Node ids are reduced to `[A-Za-z0-9_]` so a tag parses unambiguously.
fn sanitize(s: &str) -> String {
    let out: String = s
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .take(64)
        .collect();
    if out.is_empty() {
        "local".to_string()
    } else {
        out
    }
}

impl Owner {
    pub fn new(node: &str, boot: &str) -> Self {
        Self {
            node: sanitize(node),
            boot: sanitize(boot),
        }
    }

    /// `{node}.{boot}`
    pub fn tag(&self) -> String {
        format!("{}.{}", self.node, self.boot)
    }

    /// This process: node from `RAISIN_NODE_ID` (else `HOSTNAME`, else
    /// `local`), a boot id fresh for every process start.
    pub fn current() -> &'static Owner {
        static OWNER: OnceLock<Owner> = OnceLock::new();
        OWNER.get_or_init(|| {
            let node = std::env::var("RAISIN_NODE_ID")
                .or_else(|_| std::env::var("HOSTNAME"))
                .unwrap_or_else(|_| "local".to_string());
            Owner::new(&node, &nanoid::nanoid!(12, &ALNUM))
        })
    }

    /// Is something tagged `tag` provably dead from this process's view: the
    /// same node, an earlier boot?
    pub fn outlived(&self, tag: &str) -> bool {
        match tag.split_once('.') {
            Some((node, boot)) => node == self.node && boot != self.boot,
            None => false,
        }
    }
}

/// `{tag}--{rest}` → `(tag, rest)`.
pub(crate) fn split_tagged(name: &str) -> Option<(&str, &str)> {
    let (tag, rest) = name.split_once("--")?;
    tag.contains('.').then_some((tag, rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_earlier_boot_of_this_node_is_dead() {
        let me = Owner::new("node-a", "boot2");
        assert!(me.outlived(&Owner::new("node-a", "boot1").tag()));
        assert!(!me.outlived(&me.tag()));
        assert!(!me.outlived(&Owner::new("node-b", "boot1").tag()));
        assert!(!me.outlived("garbage"));
        assert_eq!(
            Owner::new("node-a.example.com", "x").node,
            "node_a_example_com"
        );
        assert_eq!(
            split_tagged("n.b--pkgbuild-1.part"),
            Some(("n.b", "pkgbuild-1.part"))
        );
        assert_eq!(split_tagged("legacy.part"), None);
    }
}
