//! Size-capped, rotating log file.
//!
//! The server used to log only to stdout, and the CLI appended stdout to
//! `~/.raisindb/server.log` for the life of the installation. Nothing ever
//! rotated it; one developer's reached 3.7 GB. With `--log-file` the server
//! owns the file: when it would grow past `max_bytes` it is renamed to
//! `<file>.1` (shifting `.1` → `.2` …) and the oldest beyond `max_files` is
//! deleted, so the total stays under `max_bytes × (max_files + 1)`.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

struct State {
    path: PathBuf,
    file: File,
    size: u64,
    max_bytes: u64,
    max_files: usize,
}

fn rotated(path: &Path, n: usize) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(format!(".{n}"));
    PathBuf::from(s)
}

fn open_append(path: &Path) -> io::Result<(File, u64)> {
    let file = OpenOptions::new().create(true).append(true).open(path)?;
    let size = file.metadata().map(|m| m.len()).unwrap_or(0);
    Ok((file, size))
}

impl State {
    fn rotate(&mut self) -> io::Result<()> {
        self.file.flush()?;
        if self.max_files == 0 {
            // Keep nothing: start the file over.
            let file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&self.path)?;
            self.file = file;
            self.size = 0;
            return Ok(());
        }
        let _ = fs::remove_file(rotated(&self.path, self.max_files));
        for n in (1..self.max_files).rev() {
            let from = rotated(&self.path, n);
            if from.exists() {
                fs::rename(&from, rotated(&self.path, n + 1))?;
            }
        }
        fs::rename(&self.path, rotated(&self.path, 1))?;
        let (file, size) = open_append(&self.path)?;
        self.file = file;
        self.size = size;
        Ok(())
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        if self.size > 0 && self.size + buf.len() as u64 > self.max_bytes {
            // A failed rotation must not lose the log line: keep appending to
            // the current file and try again on the next write.
            if let Err(e) = self.rotate() {
                eprintln!("log rotation failed for {}: {e}", self.path.display());
            }
        }
        self.file.write_all(buf)?;
        self.size += buf.len() as u64;
        Ok(())
    }
}

/// A `MakeWriter` for `tracing_subscriber::fmt` that rotates by size.
#[derive(Clone)]
pub struct RotatingFile {
    state: Arc<Mutex<State>>,
}

impl RotatingFile {
    pub fn open(path: impl Into<PathBuf>, max_bytes: u64, max_files: usize) -> io::Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        let (file, size) = open_append(&path)?;
        let mut state = State {
            path,
            file,
            size,
            max_bytes: max_bytes.max(1024),
            max_files,
        };
        // A file left over-size by a previous run (or by an older build that
        // did not rotate at all) is rotated away before we add to it.
        if state.size > state.max_bytes {
            state.rotate()?;
        }
        Ok(Self {
            state: Arc::new(Mutex::new(state)),
        })
    }
}

/// One writer handle; every `write` appends a whole formatted event.
pub struct RotatingWriter {
    state: Arc<Mutex<State>>,
}

impl Write for RotatingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        state.write_all(buf)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        state.file.flush()
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for RotatingFile {
    type Writer = RotatingWriter;

    fn make_writer(&'a self) -> Self::Writer {
        RotatingWriter {
            state: self.state.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(log: &RotatingFile, line: &str) {
        use tracing_subscriber::fmt::MakeWriter;
        log.make_writer().write_all(line.as_bytes()).unwrap();
    }

    #[test]
    fn rotates_at_the_size_cap_and_keeps_at_most_max_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.log");
        let log = RotatingFile::open(&path, 1024, 2).unwrap();
        let line = format!("{}\n", "x".repeat(299));
        for _ in 0..20 {
            write(&log, &line);
        }
        let size = |p: &Path| fs::metadata(p).map(|m| m.len()).unwrap_or(0);
        assert!(size(&path) <= 1024, "live file over the cap");
        assert!(rotated(&path, 1).exists());
        assert!(rotated(&path, 2).exists());
        assert!(!rotated(&path, 3).exists(), "only max_files are kept");
        assert!(size(&rotated(&path, 1)) <= 1024);
    }

    #[test]
    fn an_oversized_file_from_a_previous_run_is_rotated_on_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.log");
        fs::write(&path, vec![b'y'; 4096]).unwrap();
        let _log = RotatingFile::open(&path, 1024, 3).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().len(), 0);
        assert_eq!(fs::metadata(rotated(&path, 1)).unwrap().len(), 4096);
    }
}
