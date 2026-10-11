//! Object-storage [`BlobWriter`]: a multipart upload completed at commit.
//!
//! Bytes are buffered into one part (16 MiB) at a time; a full part is
//! uploaded while the next one fills, so at most two parts are in memory
//! whatever the object's size. A blob smaller than one part never starts a
//! multipart upload at all and is sent with a single `PutObject` at commit.
//! `abort` (and dropping an unfinished writer) aborts the multipart upload,
//! so no object and no billable parts are left behind. A killed process's
//! uploads are aborted by [`sweep_s3_multipart`] — only those this node
//! journaled in an earlier boot, never by listing the bucket: a shared
//! bucket holds other services' uploads and other nodes' live ones.

use crate::owner::Owner;
use crate::writer::{BlobWriter, BoxFuture};
use crate::StoredObject;
use aws_sdk_s3 as s3;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::{CompletedMultipartUpload, CompletedPart};
use chrono::Utc;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::AsyncWrite;

/// Part size. S3 requires ≥ 5 MiB for every part but the last, and allows
/// 10 000 parts — 16 MiB parts cover objects up to ~156 GiB.
pub(crate) const PART_SIZE: usize = 16 * 1024 * 1024;

type PartFuture = BoxFuture<'static, anyhow::Result<(String, CompletedPart)>>;

pub(crate) struct S3BlobWriter {
    client: s3::Client,
    bucket: String,
    key: String,
    url: String,
    name: Option<String>,
    content_type: Option<String>,
    buf: Vec<u8>,
    upload_id: Option<String>,
    parts: Vec<CompletedPart>,
    pending: Option<PartFuture>,
    written: u64,
    finished: bool,
    journal: Option<PathBuf>,
}

/// `{journal}/{node}/{boot}`: this process's journal directory.
pub(crate) fn journal_dir(root: Option<&Path>, owner: &Owner) -> Option<PathBuf> {
    root.map(|r| r.join(&owner.node).join(&owner.boot))
}

fn journal_entry(dir: &Path, upload_id: &str) -> PathBuf {
    // Upload ids are opaque provider strings; keep the file name tame.
    let safe: String = upload_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    dir.join(safe)
}

fn forget_upload(journal: &Option<PathBuf>, upload_id: &str) {
    if let Some(dir) = journal {
        let _ = std::fs::remove_file(journal_entry(dir, upload_id));
    }
}

impl S3BlobWriter {
    pub(crate) fn new(
        client: s3::Client,
        bucket: String,
        key: String,
        url: String,
        name: Option<String>,
        content_type: Option<String>,
        journal: Option<PathBuf>,
    ) -> Self {
        Self {
            journal,
            client,
            bucket,
            key,
            url,
            name,
            content_type,
            buf: Vec::with_capacity(PART_SIZE),
            upload_id: None,
            parts: Vec::new(),
            pending: None,
            written: 0,
            finished: false,
        }
    }

    /// Start uploading the buffered bytes as the next part, creating the
    /// multipart upload first if this is the first part.
    fn start_part(&mut self) {
        let body = std::mem::replace(&mut self.buf, Vec::with_capacity(PART_SIZE));
        let part_number = self.parts.len() as i32 + 1;
        let client = self.client.clone();
        let bucket = self.bucket.clone();
        let key = self.key.clone();
        let content_type = self.content_type.clone();
        let upload_id = self.upload_id.clone();
        let journal = self.journal.clone();
        self.pending = Some(Box::pin(async move {
            let upload_id = match upload_id {
                Some(id) => id,
                None => {
                    let id = client
                        .create_multipart_upload()
                        .bucket(&bucket)
                        .key(&key)
                        .set_content_type(content_type)
                        .send()
                        .await?
                        .upload_id()
                        .map(str::to_string)
                        .ok_or_else(|| anyhow::anyhow!("multipart upload returned no upload id"))?;
                    // Journaled the moment it exists, so a crash from here on
                    // leaves something a later boot of this node can abort.
                    if let Some(dir) = &journal {
                        std::fs::create_dir_all(dir)?;
                        std::fs::write(journal_entry(dir, &id), key.as_bytes())?;
                    }
                    id
                }
            };
            let out = client
                .upload_part()
                .bucket(&bucket)
                .key(&key)
                .upload_id(&upload_id)
                .part_number(part_number)
                .body(ByteStream::from(body))
                .send()
                .await?;
            let part = CompletedPart::builder()
                .part_number(part_number)
                .set_e_tag(out.e_tag().map(str::to_string))
                .build();
            Ok((upload_id, part))
        }));
    }

    fn poll_pending(&mut self, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        if let Some(fut) = self.pending.as_mut() {
            let outcome = std::task::ready!(fut.as_mut().poll(cx));
            self.pending = None;
            match outcome {
                Ok((upload_id, part)) => {
                    self.upload_id = Some(upload_id);
                    self.parts.push(part);
                }
                Err(e) => return Poll::Ready(Err(std::io::Error::other(e))),
            }
        }
        Poll::Ready(Ok(()))
    }

    async fn drain_pending(&mut self) -> std::io::Result<()> {
        std::future::poll_fn(|cx| self.poll_pending(cx)).await
    }
}

impl AsyncWrite for S3BlobWriter {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let this = &mut *self;
        loop {
            std::task::ready!(this.poll_pending(cx))?;
            if this.buf.len() < PART_SIZE {
                let n = data.len().min(PART_SIZE - this.buf.len());
                this.buf.extend_from_slice(&data[..n]);
                this.written += n as u64;
                return Poll::Ready(Ok(n));
            }
            this.start_part();
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        self.poll_pending(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        self.poll_pending(cx)
    }
}

impl BlobWriter for S3BlobWriter {
    fn key(&self) -> &str {
        &self.key
    }

    fn bytes_written(&self) -> u64 {
        self.written
    }

    fn commit(mut self: Box<Self>) -> BoxFuture<'static, anyhow::Result<StoredObject>> {
        Box::pin(async move {
            self.drain_pending().await?;
            if self.upload_id.is_none() {
                // Smaller than one part: one PutObject, no multipart upload.
                let body = std::mem::take(&mut self.buf);
                self.client
                    .put_object()
                    .bucket(&self.bucket)
                    .key(&self.key)
                    .set_content_type(self.content_type.clone())
                    .body(ByteStream::from(body))
                    .send()
                    .await?;
            } else {
                if !self.buf.is_empty() {
                    self.start_part();
                    self.drain_pending().await?;
                }
                let upload_id = self.upload_id.clone().unwrap_or_default();
                self.client
                    .complete_multipart_upload()
                    .bucket(&self.bucket)
                    .key(&self.key)
                    .upload_id(&upload_id)
                    .multipart_upload(
                        CompletedMultipartUpload::builder()
                            .set_parts(Some(std::mem::take(&mut self.parts)))
                            .build(),
                    )
                    .send()
                    .await?;
                forget_upload(&self.journal, &upload_id);
            }
            self.finished = true;
            let now = Utc::now();
            Ok(StoredObject {
                key: self.key.clone(),
                url: self.url.clone(),
                name: self.name.clone(),
                size: self.written as i64,
                mime_type: self.content_type.clone(),
                created_at: now,
                updated_at: now,
            })
        })
    }

    fn abort(mut self: Box<Self>) -> BoxFuture<'static, anyhow::Result<()>> {
        Box::pin(async move {
            // A part in flight may still create the upload; let it settle so
            // its id is known and the abort below covers it.
            let _ = self.drain_pending().await;
            self.finished = true;
            if let Some(upload_id) = self.upload_id.take() {
                self.client
                    .abort_multipart_upload()
                    .bucket(&self.bucket)
                    .key(&self.key)
                    .upload_id(&upload_id)
                    .send()
                    .await?;
                forget_upload(&self.journal, &upload_id);
            }
            Ok(())
        })
    }
}

impl Drop for S3BlobWriter {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        let Some(upload_id) = self.upload_id.take() else {
            return;
        };
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let client = self.client.clone();
        let bucket = self.bucket.clone();
        let key = self.key.clone();
        let journal = self.journal.clone();
        handle.spawn(async move {
            if client
                .abort_multipart_upload()
                .bucket(bucket)
                .key(key)
                .upload_id(&upload_id)
                .send()
                .await
                .is_ok()
            {
                forget_upload(&journal, &upload_id);
            }
        });
    }
}

/// What a sweep would abort: every upload this node journaled under a boot
/// other than `owner.boot`, as `(journal file, key, upload id)`.
pub(crate) fn dead_uploads(root: &Path, owner: &Owner) -> Vec<(PathBuf, String, String)> {
    let mut out = Vec::new();
    let node_dir = root.join(&owner.node);
    let Ok(boots) = std::fs::read_dir(&node_dir) else {
        return out;
    };
    for boot in boots.flatten() {
        if boot.file_name().to_string_lossy() == owner.boot {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(boot.path()) else {
            continue;
        };
        for e in entries.flatten() {
            let upload_id = e.file_name().to_string_lossy().into_owned();
            if let Ok(key) = std::fs::read_to_string(e.path()) {
                out.push((e.path(), key, upload_id));
            }
        }
    }
    out
}

/// Abort the multipart uploads this node journaled in an earlier boot.
/// Never lists the bucket: without a journal it does nothing.
pub(crate) async fn sweep_s3_multipart(
    client: &s3::Client,
    bucket: &str,
    journal: Option<&Path>,
    owner: &Owner,
) -> anyhow::Result<usize> {
    let Some(root) = journal else {
        return Ok(0);
    };
    let mut removed = 0;
    for (file, key, upload_id) in dead_uploads(root, owner) {
        if key.is_empty() || key.starts_with('/') {
            continue;
        }
        let aborted = client
            .abort_multipart_upload()
            .bucket(bucket)
            .key(&key)
            .upload_id(&upload_id)
            .send()
            .await;
        // Gone already (completed or aborted elsewhere) counts as done too.
        let done = match &aborted {
            Ok(_) => true,
            Err(e) => e
                .as_service_error()
                .map(|se| se.is_no_such_upload())
                .unwrap_or(false),
        };
        if done {
            let _ = std::fs::remove_file(&file);
            removed += aborted.is_ok() as usize;
        }
    }
    // Empty boot directories of this node go too.
    if let Ok(boots) = std::fs::read_dir(root.join(&owner.node)) {
        for b in boots.flatten() {
            if b.file_name().to_string_lossy() != owner.boot {
                let _ = std::fs::remove_dir(b.path());
            }
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_this_nodes_earlier_boots_are_swept() {
        let dir = tempfile::tempdir().unwrap();
        let me = Owner::new("node-a", "boot2");
        for (o, id) in [
            (Owner::new("node-a", "boot1"), "dead-upload"),
            (me.clone(), "live-upload"),
            (Owner::new("node-b", "boot1"), "other-node-upload"),
        ] {
            let d = journal_dir(Some(dir.path()), &o).unwrap();
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(journal_entry(&d, id), format!("t/2026/01/01/{id}.rap")).unwrap();
        }
        let dead = dead_uploads(dir.path(), &me);
        assert_eq!(dead.len(), 1);
        assert_eq!(dead[0].2, "dead-upload");
        assert_eq!(dead[0].1, "t/2026/01/01/dead-upload.rap");
    }
}
