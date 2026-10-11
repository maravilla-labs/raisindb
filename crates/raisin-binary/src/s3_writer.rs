//! Object-storage [`BlobWriter`]: a multipart upload completed at commit.
//!
//! Bytes are buffered into one part (16 MiB) at a time; a full part is
//! uploaded while the next one fills, so at most two parts are in memory
//! whatever the object's size. A blob smaller than one part never starts a
//! multipart upload at all and is sent with a single `PutObject` at commit.
//! `abort` (and dropping an unfinished writer) aborts the multipart upload,
//! so no object and no billable parts are left behind; a killed process's
//! uploads are aborted by [`sweep_s3_multipart`].

use crate::writer::{BlobWriter, BoxFuture};
use crate::StoredObject;
use aws_sdk_s3 as s3;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::{CompletedMultipartUpload, CompletedPart};
use chrono::Utc;
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
}

impl S3BlobWriter {
    pub(crate) fn new(
        client: s3::Client,
        bucket: String,
        key: String,
        url: String,
        name: Option<String>,
        content_type: Option<String>,
    ) -> Self {
        Self {
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
        self.pending = Some(Box::pin(async move {
            let upload_id = match upload_id {
                Some(id) => id,
                None => client
                    .create_multipart_upload()
                    .bucket(&bucket)
                    .key(&key)
                    .set_content_type(content_type)
                    .send()
                    .await?
                    .upload_id()
                    .map(str::to_string)
                    .ok_or_else(|| anyhow::anyhow!("multipart upload returned no upload id"))?,
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
                    .upload_id(upload_id)
                    .send()
                    .await?;
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
        handle.spawn(async move {
            let _ = client
                .abort_multipart_upload()
                .bucket(bucket)
                .key(key)
                .upload_id(upload_id)
                .send()
                .await;
        });
    }
}

/// Abort multipart uploads started more than `older_than` ago.
pub(crate) async fn sweep_s3_multipart(
    client: &s3::Client,
    bucket: &str,
    older_than: std::time::Duration,
) -> anyhow::Result<usize> {
    let cutoff = std::time::SystemTime::now()
        .checked_sub(older_than)
        .unwrap_or(std::time::UNIX_EPOCH);
    let mut removed = 0;
    let mut key_marker: Option<String> = None;
    let mut upload_marker: Option<String> = None;
    loop {
        let page = client
            .list_multipart_uploads()
            .bucket(bucket)
            .set_key_marker(key_marker.clone())
            .set_upload_id_marker(upload_marker.clone())
            .send()
            .await?;
        for upload in page.uploads() {
            let started = upload
                .initiated()
                .and_then(|t| std::time::SystemTime::try_from(*t).ok());
            let stale = started.map(|t| t <= cutoff).unwrap_or(false);
            if let (true, Some(key), Some(id)) = (stale, upload.key(), upload.upload_id()) {
                if client
                    .abort_multipart_upload()
                    .bucket(bucket)
                    .key(key)
                    .upload_id(id)
                    .send()
                    .await
                    .is_ok()
                {
                    removed += 1;
                }
            }
        }
        if page.is_truncated() != Some(true) {
            break;
        }
        key_marker = page.next_key_marker().map(str::to_string);
        upload_marker = page.next_upload_id_marker().map(str::to_string);
    }
    Ok(removed)
}
