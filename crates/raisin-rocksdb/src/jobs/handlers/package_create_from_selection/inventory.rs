//! The package inventory (E8): one JSON line per archived node, sorted.
//!
//! The selection is walked in document order (pre-order DFS by
//! `tree_order`), but the inventory is sorted by `(workspace, path)` in byte
//! order — the key that is stable across rebuilds (an editorial reorder
//! changes `tree_order`, never the path) and the one a diff merge-joins on.
//! So lines are pushed in walk order and sorted externally: fixed-size runs
//! are sorted in memory and spilled, then k-way merged at seal. Memory is one
//! run, whatever the number of nodes.

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};

/// Default in-memory run size before a spill.
pub(super) const DEFAULT_RUN_BYTES: usize = 16 * 1024 * 1024;

/// One inventory line, exactly as written to `inventory.jsonl`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct InventoryLine {
    pub ws: String,
    pub path: String,
    pub id: String,
    #[serde(rename = "type")]
    pub node_type: String,
    /// `sha256:<hex>` over the node as serialised into the archive.
    pub hash: String,
    /// Size of the serialised node.
    pub bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub bin: Option<BinaryRef>,
    /// The node names an embedded file that was NOT packed: `missing` (no
    /// such blob) or `foreign_key` (the key is not this tenant's, never read).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub bin_skipped: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct BinaryRef {
    pub sha256: String,
    pub bytes: u64,
}

/// The sort key of a serialised line, read without decoding the rest.
#[derive(Deserialize)]
struct KeyOnly<'a> {
    #[serde(borrow)]
    ws: std::borrow::Cow<'a, str>,
    #[serde(borrow)]
    path: std::borrow::Cow<'a, str>,
}

fn key_of(line: &str) -> std::io::Result<(String, String)> {
    let k: KeyOnly<'_> = serde_json::from_str(line).map_err(std::io::Error::other)?;
    Ok((k.ws.into_owned(), k.path.into_owned()))
}

fn cmp_key(a: &(String, String), b: &(String, String)) -> Ordering {
    a.0.as_bytes()
        .cmp(b.0.as_bytes())
        .then_with(|| a.1.as_bytes().cmp(b.1.as_bytes()))
}

/// Accumulates lines in walk order; [`finish`](Self::finish) yields them sorted.
pub(super) struct InventorySorter {
    spill_dir: PathBuf,
    run_bytes: usize,
    buf: Vec<((String, String), String)>,
    buf_bytes: usize,
    runs: Vec<PathBuf>,
    lines: u64,
}

impl InventorySorter {
    pub(super) fn new(spill_dir: &Path, run_bytes: usize) -> Self {
        Self {
            spill_dir: spill_dir.to_path_buf(),
            run_bytes: run_bytes.max(1),
            buf: Vec::new(),
            buf_bytes: 0,
            runs: Vec::new(),
            lines: 0,
        }
    }

    pub(super) fn lines(&self) -> u64 {
        self.lines
    }

    pub(super) async fn push(&mut self, line: &InventoryLine) -> std::io::Result<()> {
        let json = serde_json::to_string(line).map_err(std::io::Error::other)?;
        self.buf_bytes += json.len() + line.ws.len() + line.path.len() + 64;
        self.buf.push(((line.ws.clone(), line.path.clone()), json));
        self.lines += 1;
        if self.buf_bytes >= self.run_bytes {
            self.spill().await?;
        }
        Ok(())
    }

    async fn spill(&mut self) -> std::io::Result<()> {
        if self.buf.is_empty() {
            return Ok(());
        }
        self.buf.sort_by(|a, b| cmp_key(&a.0, &b.0));
        tokio::fs::create_dir_all(&self.spill_dir).await?;
        let path = self.spill_dir.join(format!("{}.invrun", nanoid::nanoid!()));
        // Registered before writing, so a failure part-way still cleans it up.
        self.runs.push(path.clone());
        let mut w = BufWriter::with_capacity(256 * 1024, tokio::fs::File::create(&path).await?);
        for (_, json) in self.buf.drain(..) {
            w.write_all(json.as_bytes()).await?;
            w.write_all(b"\n").await?;
        }
        w.flush().await?;
        self.buf_bytes = 0;
        Ok(())
    }

    /// Every line, sorted by `(ws, path)`, as a pull-based reader.
    pub(super) async fn finish(mut self) -> std::io::Result<SortedLines> {
        if self.runs.is_empty() {
            // Everything fit in one run: sort in memory, no file at all.
            self.buf.sort_by(|a, b| cmp_key(&a.0, &b.0));
            let lines: Vec<String> = std::mem::take(&mut self.buf)
                .into_iter()
                .map(|(_, json)| json)
                .collect();
            return Ok(SortedLines {
                memory: lines.into_iter(),
                readers: Vec::new(),
                heap: BinaryHeap::new(),
                _runs: self,
            });
        }
        self.spill().await?;
        let mut readers = Vec::with_capacity(self.runs.len());
        for path in &self.runs {
            let f = tokio::fs::File::open(path).await?;
            readers.push(BufReader::with_capacity(64 * 1024, f).lines());
        }
        let mut heap = BinaryHeap::with_capacity(readers.len());
        for (run, r) in readers.iter_mut().enumerate() {
            if let Some(line) = r.next_line().await? {
                heap.push(Head {
                    key: key_of(&line)?,
                    line,
                    run,
                });
            }
        }
        Ok(SortedLines {
            memory: Vec::new().into_iter(),
            readers,
            heap,
            _runs: self,
        })
    }
}

struct Head {
    key: (String, String),
    line: String,
    run: usize,
}

impl PartialEq for Head {
    fn eq(&self, o: &Self) -> bool {
        self.cmp(o) == Ordering::Equal
    }
}

impl Eq for Head {}

impl PartialOrd for Head {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

impl Ord for Head {
    // Reversed: BinaryHeap is a max-heap, the merge wants the minimum. Ties
    // break on run index, so equal keys keep push order.
    fn cmp(&self, o: &Self) -> Ordering {
        cmp_key(&o.key, &self.key).then_with(|| o.run.cmp(&self.run))
    }
}

/// The sorted lines of a finished [`InventorySorter`]: a k-way merge over
/// its spilled runs. Holds the sorter, so the runs are removed when this is
/// dropped.
pub(super) struct SortedLines {
    memory: std::vec::IntoIter<String>,
    readers: Vec<tokio::io::Lines<BufReader<tokio::fs::File>>>,
    heap: BinaryHeap<Head>,
    _runs: InventorySorter,
}

impl SortedLines {
    /// The next line, newline included.
    pub(super) async fn next(&mut self) -> std::io::Result<Option<String>> {
        if let Some(mut line) = self.memory.next() {
            line.push('\n');
            return Ok(Some(line));
        }
        let Some(Head { line, run, .. }) = self.heap.pop() else {
            return Ok(None);
        };
        if let Some(next) = self.readers[run].next_line().await? {
            self.heap.push(Head {
                key: key_of(&next)?,
                line: next,
                run,
            });
        }
        let mut out = line;
        out.push('\n');
        Ok(Some(out))
    }
}

impl Drop for InventorySorter {
    fn drop(&mut self) {
        for path in &self.runs {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{seq::SliceRandom, SeedableRng};

    fn line(ws: &str, path: &str) -> InventoryLine {
        InventoryLine {
            ws: ws.into(),
            path: path.into(),
            id: format!("id-{ws}-{path}"),
            node_type: "raisin:Folder".into(),
            hash: "sha256:00".into(),
            bytes: path.len() as u64,
            bin: (path.len() % 3 == 0).then(|| BinaryRef {
                sha256: "ab".into(),
                bytes: 7,
            }),
            bin_skipped: None,
        }
    }

    async fn sorted(lines: &[InventoryLine], run_bytes: usize, dir: &Path) -> Vec<InventoryLine> {
        let mut s = InventorySorter::new(dir, run_bytes);
        for l in lines {
            s.push(l).await.unwrap();
        }
        let mut out = Vec::new();
        let mut sorted = s.finish().await.unwrap();
        while let Some(l) = sorted.next().await.unwrap() {
            out.push(l);
        }
        drop(sorted);
        assert_eq!(out.len(), lines.len());
        out.into_iter()
            .map(|l| {
                assert!(l.ends_with('\n'));
                serde_json::from_str(l.trim_end()).unwrap()
            })
            .collect()
    }

    #[tokio::test]
    async fn the_external_sort_matches_an_in_memory_sort() {
        let mut rng = rand::rngs::StdRng::seed_from_u64(3);
        let mut lines = Vec::new();
        for ws in ["stories", "assets", "nodetypes", "Zeta", "ä-ws"] {
            for i in 0..400 {
                // Paths whose byte order differs from DFS order and from a
                // naive per-segment order: '/a/b' vs '/a-b' vs '/a/b/c'.
                let p = match i % 4 {
                    0 => format!("/a/{i}"),
                    1 => format!("/a-{i}"),
                    2 => format!("/a/{i}/c"),
                    _ => format!("/Ü/{i}"),
                };
                lines.push(line(ws, &p));
            }
        }
        lines.shuffle(&mut rng);
        let mut expected = lines.clone();
        expected.sort_by(|a, b| {
            a.ws.as_bytes()
                .cmp(b.ws.as_bytes())
                .then(a.path.as_bytes().cmp(b.path.as_bytes()))
        });

        let dir = tempfile::tempdir().unwrap();
        // One run (in memory), and many small runs (spilled + merged).
        for run_bytes in [DEFAULT_RUN_BYTES, 4096, 1] {
            let got = sorted(&lines, run_bytes, dir.path()).await;
            assert_eq!(got, expected, "run_bytes={run_bytes}");
        }
        // Every spilled run was removed.
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn an_abandoned_sorter_removes_its_runs() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut s = InventorySorter::new(dir.path(), 1);
            for i in 0..10 {
                s.push(&line("ws", &format!("/{i}"))).await.unwrap();
            }
            assert!(std::fs::read_dir(dir.path()).unwrap().count() > 0);
        }
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn the_line_format_is_the_documented_one() {
        let l = InventoryLine {
            ws: "stories".into(),
            path: "/acme/home".into(),
            id: "n1".into(),
            node_type: "studio:Page".into(),
            hash: "sha256:abc".into(),
            bytes: 2311,
            bin: Some(BinaryRef {
                sha256: "def".into(),
                bytes: 834221,
            }),
            bin_skipped: None,
        };
        assert_eq!(
            serde_json::to_string(&l).unwrap(),
            r#"{"ws":"stories","path":"/acme/home","id":"n1","type":"studio:Page","hash":"sha256:abc","bytes":2311,"bin":{"sha256":"def","bytes":834221}}"#
        );
    }
}
