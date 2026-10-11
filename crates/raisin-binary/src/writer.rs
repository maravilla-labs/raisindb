//! Incremental, atomically committed writes into a [`BinaryStorage`].
//!
//! [`BinaryStorage::put_stream`] needs the whole body to be a `Stream` up
//! front. A producer that generates bytes as it goes — a package archive
//! written entry by entry — needs the opposite shape: a sink it can write into
//! for as long as it likes, then either *publish* or *throw away*. That is a
//! [`BlobWriter`].
//!
//! The contract every backend keeps:
//!
//! - Nothing is visible under the final key until [`BlobWriter::commit`]
//!   returns. The filesystem writes `<root>/.partial/<id>` and renames it into
//!   place (atomic on one filesystem); object storage uses a multipart upload
//!   completed at commit.
//! - [`BlobWriter::abort`] — and dropping a writer that was neither committed
//!   nor aborted — leaves no object behind. A process that dies outright leaves
//!   a partial that [`BinaryStorage::sweep_partials`] removes on the next boot.
//!
//! [`BinaryStorage`]: crate::BinaryStorage
//! [`BinaryStorage::put_stream`]: crate::BinaryStorage::put_stream
//! [`BinaryStorage::sweep_partials`]: crate::BinaryStorage::sweep_partials

use crate::owner::Owner;
use crate::StoredObject;
use chrono::{DateTime, Utc};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::AsyncWrite;

/// A boxed, sendable future, so [`BlobWriter`] stays object safe.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Directory under a filesystem store's root that holds uncommitted writes.
///
/// It is excluded from [`list_blobs`](crate::BinaryStorage::list_blobs), so
/// garbage collection never mistakes an in-flight write for an unreferenced
/// blob.
pub const PARTIAL_DIR: &str = ".partial";

/// What the caller knows about a blob before writing it.
#[derive(Debug, Clone, Default)]
pub struct WriteHint {
    pub content_type: Option<String>,
    pub ext: Option<String>,
    pub original_name: Option<String>,
    pub tenant_context: Option<String>,
    pub size_hint: Option<u64>,
    /// A stable name for the uncommitted write, so a later attempt of the
    /// same work can find (and overwrite or discard) what an interrupted one
    /// left — see [`BinaryStorage::discard_partial`](crate::BinaryStorage::discard_partial).
    /// Only `[A-Za-z0-9_-]` is kept. Without one the name is random.
    pub partial_id: Option<String>,
}

impl WriteHint {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn content_type(mut self, v: impl Into<String>) -> Self {
        self.content_type = Some(v.into());
        self
    }

    pub fn ext(mut self, v: impl Into<String>) -> Self {
        self.ext = Some(v.into());
        self
    }

    pub fn original_name(mut self, v: impl Into<String>) -> Self {
        self.original_name = Some(v.into());
        self
    }

    pub fn tenant(mut self, v: Option<impl Into<String>>) -> Self {
        self.tenant_context = v.map(Into::into);
        self
    }

    pub fn partial_id(mut self, v: impl Into<String>) -> Self {
        self.partial_id = Some(v.into());
        self
    }
}

fn sanitized_id(partial_id: &str) -> String {
    partial_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect()
}

/// The file name of a partial: `{owner tag}--{stable id or random}.part`.
pub(crate) fn partial_file_name(owner: &Owner, partial_id: Option<&str>) -> String {
    let id = partial_id
        .map(sanitized_id)
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| nanoid::nanoid!());
    format!("{}--{}.part", owner.tag(), id)
}

/// Does `file_name` hold the partial with stable id `partial_id`, whoever
/// wrote it (a retry after a restart runs under a new boot id)?
pub(crate) fn is_partial_named(file_name: &str, partial_id: &str) -> bool {
    let id = sanitized_id(partial_id);
    if id.is_empty() {
        return false;
    }
    let wanted = format!("{id}.part");
    match crate::owner::split_tagged(file_name) {
        Some((_, rest)) => rest == wanted,
        None => file_name == wanted,
    }
}

/// Size and age of one stored blob, from [`head`](crate::BinaryStorage::head).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobMeta {
    pub size: u64,
    pub modified: Option<DateTime<Utc>>,
}

/// Free and total bytes on the volume a store writes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageSpace {
    pub available: u64,
    pub total: u64,
}

/// An open, uncommitted write. See the [module docs](self).
pub trait BlobWriter: AsyncWrite + Send + Unpin {
    /// The key the blob will have once committed.
    fn key(&self) -> &str;

    /// Bytes accepted so far.
    fn bytes_written(&self) -> u64;

    /// Publish the blob under [`key`](Self::key). Flushes first.
    fn commit(self: Box<Self>) -> BoxFuture<'static, anyhow::Result<StoredObject>>;

    /// Discard everything written. Never leaves an object behind.
    fn abort(self: Box<Self>) -> BoxFuture<'static, anyhow::Result<()>>;
}

/// Filesystem [`BlobWriter`]: a file under [`PARTIAL_DIR`], renamed at commit.
pub(crate) struct FsBlobWriter {
    file: Option<tokio::fs::File>,
    partial: PathBuf,
    final_path: PathBuf,
    key: String,
    url: String,
    name: Option<String>,
    mime_type: Option<String>,
    written: u64,
    finished: bool,
}

impl FsBlobWriter {
    pub(crate) async fn open(
        base_dir: &Path,
        owner: &Owner,
        key: String,
        url: String,
        hint: &WriteHint,
    ) -> anyhow::Result<Self> {
        let partial_dir = base_dir.join(PARTIAL_DIR);
        tokio::fs::create_dir_all(&partial_dir).await?;
        // `create` truncates: a stable id reused by a retry starts clean.
        let partial = partial_dir.join(partial_file_name(owner, hint.partial_id.as_deref()));
        let file = tokio::fs::File::create(&partial).await?;
        Ok(Self {
            file: Some(file),
            partial,
            final_path: base_dir.join(&key),
            key,
            url,
            name: hint.original_name.clone(),
            mime_type: hint.content_type.clone(),
            written: 0,
            finished: false,
        })
    }

    fn file(&mut self) -> std::io::Result<&mut tokio::fs::File> {
        self.file
            .as_mut()
            .ok_or_else(|| std::io::Error::other("blob writer already finished"))
    }
}

impl AsyncWrite for FsBlobWriter {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let this = &mut *self;
        let file = match this.file() {
            Ok(f) => f,
            Err(e) => return Poll::Ready(Err(e)),
        };
        match Pin::new(file).poll_write(cx, buf) {
            Poll::Ready(Ok(n)) => {
                this.written += n as u64;
                Poll::Ready(Ok(n))
            }
            other => other,
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.file() {
            Ok(f) => Pin::new(f).poll_flush(cx),
            Err(e) => Poll::Ready(Err(e)),
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.file() {
            Ok(f) => Pin::new(f).poll_shutdown(cx),
            Err(e) => Poll::Ready(Err(e)),
        }
    }
}

impl BlobWriter for FsBlobWriter {
    fn key(&self) -> &str {
        &self.key
    }

    fn bytes_written(&self) -> u64 {
        self.written
    }

    fn commit(mut self: Box<Self>) -> BoxFuture<'static, anyhow::Result<StoredObject>> {
        Box::pin(async move {
            use tokio::io::AsyncWriteExt;
            let mut file = self
                .file
                .take()
                .ok_or_else(|| anyhow::anyhow!("blob writer already finished"))?;
            let outcome: anyhow::Result<()> = async {
                file.flush().await?;
                file.sync_all().await?;
                drop(file);
                if let Some(parent) = self.final_path.parent() {
                    tokio::fs::create_dir_all(parent).await?;
                }
                tokio::fs::rename(&self.partial, &self.final_path).await?;
                Ok(())
            }
            .await;
            // Whatever happened, this writer is done: on failure the Drop
            // guard removes the partial; on success there is nothing left.
            self.finished = outcome.is_ok();
            outcome?;
            let now = Utc::now();
            Ok(StoredObject {
                key: self.key.clone(),
                url: self.url.clone(),
                name: self.name.clone(),
                size: self.written as i64,
                mime_type: self.mime_type.clone(),
                created_at: now,
                updated_at: now,
            })
        })
    }

    fn abort(mut self: Box<Self>) -> BoxFuture<'static, anyhow::Result<()>> {
        Box::pin(async move {
            drop(self.file.take());
            self.finished = true;
            match tokio::fs::remove_file(&self.partial).await {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e.into()),
            }
        })
    }
}

impl Drop for FsBlobWriter {
    fn drop(&mut self) {
        if !self.finished {
            drop(self.file.take());
            let _ = std::fs::remove_file(&self.partial);
        }
    }
}

/// Remove the filesystem partials provably abandoned: those of THIS node from
/// an earlier boot (see [`crate::owner`]). Another node's partials and this
/// process's own are kept whatever their age. A partial without an owner tag
/// (written before tags existed) is removed once older than `older_than`.
pub(crate) fn sweep_fs_partials(
    base_dir: &Path,
    owner: &Owner,
    older_than: std::time::Duration,
) -> anyhow::Result<usize> {
    let dir = base_dir.join(PARTIAL_DIR);
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e.into()),
    };
    let cutoff = std::time::SystemTime::now()
        .checked_sub(older_than)
        .unwrap_or(std::time::UNIX_EPOCH);
    let mut removed = 0;
    for entry in entries {
        let entry = entry?;
        let meta = entry.metadata()?;
        if !meta.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let dead = match crate::owner::split_tagged(&name) {
            Some((tag, _)) => owner.outlived(tag),
            None => meta.modified().map(|m| m <= cutoff).unwrap_or(false),
        };
        if dead && std::fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

/// Remove every partial with stable id `partial_id`; returns whether any was.
pub(crate) fn discard_fs_partial(base_dir: &Path, partial_id: &str) -> anyhow::Result<bool> {
    let dir = base_dir.join(PARTIAL_DIR);
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.into()),
    };
    let mut any = false;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_partial_named(&name, partial_id) && std::fs::remove_file(entry.path()).is_ok() {
            any = true;
        }
    }
    Ok(any)
}
