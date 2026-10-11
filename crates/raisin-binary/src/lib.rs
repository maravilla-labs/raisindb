#[cfg(feature = "s3")]
use aws_sdk_s3 as s3;
use bytes::Bytes;
use chrono::{DateTime, Utc};
use futures_core::Stream;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    ops::Range,
    path::{Path, PathBuf},
    pin::Pin,
};

mod dyn_store;
mod hashing;
#[cfg(feature = "s3")]
mod s3_writer;
pub mod space;
mod writer;
pub mod zip_stream;

pub use dyn_store::{erase, DynBinaryStorage};
pub use hashing::{sha256_hex_to_base64, HashingWriter, StreamDigest, StreamHasher};
pub use writer::{BlobMeta, BlobWriter, BoxFuture, StorageSpace, WriteHint, PARTIAL_DIR};

/// A boxed byte stream returned by [`BinaryStorage::get_stream`].
pub type ByteReadStream = Pin<Box<dyn Stream<Item = std::io::Result<Bytes>> + Send>>;

/// Validate a storage key to prevent path traversal attacks
///
/// Returns an error if the key contains dangerous patterns like:
/// - Parent directory references (..)
/// - Absolute paths (starting with /)
/// - Windows absolute paths (containing :)
fn validate_key(key: &str) -> anyhow::Result<()> {
    // Check for empty key
    if key.is_empty() {
        anyhow::bail!("Storage key cannot be empty");
    }

    // Check for absolute paths
    if key.starts_with('/') || key.starts_with('\\') {
        anyhow::bail!("Storage key cannot be an absolute path");
    }

    // Check for Windows absolute paths (C:, D:, etc.)
    if key.contains(':') {
        anyhow::bail!("Storage key cannot contain drive letters");
    }

    // Check for parent directory references
    let path = Path::new(key);
    for component in path.components() {
        if component == std::path::Component::ParentDir {
            anyhow::bail!("Storage key cannot contain parent directory references (..)");
        }
    }

    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredObject {
    pub key: String,
    pub url: String,
    pub name: Option<String>,
    pub size: i64,
    pub mime_type: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// One stored blob, as [`BinaryStorage::list_blobs`] enumerates it.
#[derive(Debug, Clone)]
pub struct ListedBlob {
    pub key: String,
    pub size: u64,
    /// Last modification, where the backend reports one.
    pub modified: Option<DateTime<Utc>>,
}

/// Trait for binary storage backends
///
/// Provides functionality for storing, retrieving, and deleting binary data.
/// Implementations should handle tenant isolation when `tenant_context` is provided.
pub trait BinaryStorage: Send + Sync {
    /// Store a binary stream and return metadata about the stored object
    ///
    /// # Arguments
    /// * `stream` - Stream of bytes to store
    /// * `content_type` - Optional MIME type of the content
    /// * `ext` - Optional file extension
    /// * `original_name` - Optional original filename
    /// * `size_hint` - Optional size hint for optimization
    /// * `tenant_context` - Optional tenant context for multi-tenant isolation
    ///
    /// # Returns
    /// Metadata about the stored object including its key and URL
    fn put_stream<'a, S>(
        &'a self,
        stream: S,
        content_type: Option<&'a str>,
        ext: Option<&'a str>,
        original_name: Option<&'a str>,
        size_hint: Option<u64>,
        tenant_context: Option<&'a str>,
    ) -> impl std::future::Future<Output = anyhow::Result<StoredObject>> + Send
    where
        S: Stream<Item = Result<Bytes, std::io::Error>> + Send + 'a;

    /// Store bytes and return metadata about the stored object
    ///
    /// # Arguments
    /// * `bytes` - Bytes to store
    /// * `content_type` - Optional MIME type
    /// * `ext` - Optional file extension
    /// * `original_name` - Optional original filename
    /// * `tenant_context` - Optional tenant context for multi-tenant isolation
    fn put_bytes<'a>(
        &'a self,
        bytes: &'a [u8],
        content_type: Option<&'a str>,
        ext: Option<&'a str>,
        original_name: Option<&'a str>,
        tenant_context: Option<&'a str>,
    ) -> impl std::future::Future<Output = anyhow::Result<StoredObject>> + Send {
        let s = futures_util::stream::once(async move {
            Ok::<Bytes, std::io::Error>(Bytes::copy_from_slice(bytes))
        });
        self.put_stream(
            s,
            content_type,
            ext,
            original_name,
            Some(bytes.len() as u64),
            tenant_context,
        )
    }

    /// Retrieve binary data by storage key
    ///
    /// # Arguments
    /// * `key` - The storage key (validated for path traversal attacks)
    ///
    /// # Returns
    /// The binary data as Bytes
    ///
    /// # Security
    /// The key is validated to prevent path traversal attacks
    fn get(&self, key: &str) -> impl std::future::Future<Output = anyhow::Result<Bytes>> + Send;

    /// Stream binary data by storage key without buffering it all in memory.
    ///
    /// Returns the total byte length (for a `Content-Length` header) and a
    /// stream of chunks. Prefer this over [`get`](Self::get) when serving bytes
    /// straight to a client — the filesystem backend streams from disk and the
    /// S3 backend streams the object body, so neither holds the whole asset in
    /// process memory.
    ///
    /// # Security
    /// The key is validated to prevent path traversal attacks.
    fn get_stream(
        &self,
        key: &str,
    ) -> impl std::future::Future<Output = anyhow::Result<(u64, ByteReadStream)>> + Send;

    /// Delete binary data by storage key
    ///
    /// # Arguments
    /// * `key` - The storage key (validated for path traversal attacks)
    ///
    /// # Security
    /// The key is validated to prevent path traversal attacks
    fn delete(&self, key: &str) -> impl std::future::Future<Output = anyhow::Result<()>> + Send;

    /// Generate a public URL for accessing the stored object
    ///
    /// # Arguments
    /// * `key` - The storage key
    ///
    /// # Returns
    /// A URL string for accessing the object
    fn url_for(&self, key: &str) -> String;

    /// Get binary data as a file path for efficient processing.
    ///
    /// This method is **transparent to storage backend**:
    /// - **Filesystem**: Returns actual file path directly (zero-copy, `is_temp=false`)
    /// - **S3/R2/Azure**: Downloads to temp file, returns temp path (`is_temp=true`)
    ///
    /// The caller **MUST** cleanup temp files when `is_temp=true`.
    ///
    /// # Arguments
    /// * `key` - The storage key (validated for path traversal attacks)
    ///
    /// # Returns
    /// Tuple of `(PathBuf, is_temp)`:
    /// - `PathBuf` - Path to the file (actual or temporary)
    /// - `is_temp` - If `true`, caller must delete the file after use
    ///
    /// # Example
    /// ```rust,ignore
    /// let (path, is_temp) = storage.get_as_path("uploads/doc.pdf").await?;
    /// // Use the file...
    /// if is_temp {
    ///     tokio::fs::remove_file(&path).await.ok();
    /// }
    /// ```
    fn get_as_path(
        &self,
        key: &str,
    ) -> impl std::future::Future<Output = anyhow::Result<(PathBuf, bool)>> + Send;

    /// Every blob this store holds, or `None` when the backend cannot say.
    ///
    /// Garbage collection deletes the blobs no database key mentions, so a
    /// backend must only list what it owns outright. The default is `None`:
    /// an object store may share its bucket with other applications, and
    /// "unreferenced by this database" would then say nothing about an object.
    fn list_blobs(
        &self,
    ) -> impl std::future::Future<Output = anyhow::Result<Option<Vec<ListedBlob>>>> + Send {
        async { Ok(None) }
    }

    /// Open an incremental write that becomes visible only at
    /// [`BlobWriter::commit`]. See [`writer`](crate::BlobWriter) for the
    /// no-orphan contract.
    fn begin_write(
        &self,
        hint: WriteHint,
    ) -> impl std::future::Future<Output = anyhow::Result<Box<dyn BlobWriter>>> + Send;

    /// Stream the half-open byte range `range` of a blob.
    ///
    /// The range must lie within the blob (`range.end <= size`); an empty
    /// range yields an empty stream. Only the requested bytes are read.
    fn get_range(
        &self,
        key: &str,
        range: Range<u64>,
    ) -> impl std::future::Future<Output = anyhow::Result<ByteReadStream>> + Send;

    /// Size and modification time of a blob, `None` if it does not exist.
    fn head(
        &self,
        key: &str,
    ) -> impl std::future::Future<Output = anyhow::Result<Option<BlobMeta>>> + Send;

    /// Free and total space of the volume this store writes to, or `None`
    /// when the backend cannot observe it (object storage). Callers apply
    /// [`space::check_room`] to the answer.
    fn available_space(
        &self,
    ) -> impl std::future::Future<Output = anyhow::Result<Option<StorageSpace>>> + Send {
        async { Ok(None) }
    }

    /// Remove the uncommitted write named by [`WriteHint::partial_id`], if
    /// one is left over. Returns whether there was one. Object storage cannot
    /// name an upload this way and leaves it to [`sweep_partials`](Self::sweep_partials).
    fn discard_partial(
        &self,
        partial_id: &str,
    ) -> impl std::future::Future<Output = anyhow::Result<bool>> + Send {
        let _ = partial_id;
        async { Ok(false) }
    }

    /// Remove uncommitted writes older than `older_than` — what a process
    /// killed mid-write left behind. Returns how many were removed.
    fn sweep_partials(
        &self,
        older_than: std::time::Duration,
    ) -> impl std::future::Future<Output = anyhow::Result<usize>> + Send {
        let _ = older_than;
        async { Ok(0) }
    }
}

/// Check a requested range against a blob's size.
fn check_range(range: &Range<u64>, size: u64) -> anyhow::Result<()> {
    if range.start > range.end || range.end > size {
        anyhow::bail!(
            "range {}..{} is outside a blob of {} bytes",
            range.start,
            range.end,
            size
        );
    }
    Ok(())
}

pub struct FilesystemBinaryStorage {
    base_dir: PathBuf,
    url_base: Option<String>,
}

impl FilesystemBinaryStorage {
    pub fn new(base_dir: impl Into<PathBuf>, url_base: Option<String>) -> Self {
        Self {
            base_dir: base_dir.into(),
            url_base,
        }
    }

    /// Generate a storage key with optional tenant/deployment prefix
    ///
    /// Format:
    /// - Single-tenant: `2025/01/15/nanoid.ext`
    /// - Multi-tenant: `{tenant_id}/{deployment}/2025/01/15/nanoid.ext`
    fn generate_key(ext: Option<&str>, tenant_prefix: Option<&str>) -> String {
        let today = Utc::now().format("%Y/%m/%d").to_string();
        let id = nanoid::nanoid!();

        let file_part = if let Some(e) = ext {
            format!("{}.{}", id, e.trim_start_matches('.'))
        } else {
            id
        };

        if let Some(prefix) = tenant_prefix {
            format!("{}/{}/{}", prefix.trim_matches('/'), today, file_part)
        } else {
            format!("{}/{}", today, file_part)
        }
    }
}

impl BinaryStorage for FilesystemBinaryStorage {
    async fn put_stream<'a, S>(
        &'a self,
        stream: S,
        content_type: Option<&'a str>,
        ext: Option<&'a str>,
        original_name: Option<&'a str>,
        size_hint: Option<u64>,
        tenant_context: Option<&'a str>,
    ) -> anyhow::Result<StoredObject>
    where
        S: Stream<Item = Result<Bytes, std::io::Error>> + Send + 'a,
    {
        let hint = WriteHint {
            content_type: content_type.map(str::to_string),
            ext: ext.map(str::to_string),
            original_name: original_name.map(str::to_string),
            tenant_context: tenant_context.map(str::to_string),
            size_hint,
            partial_id: None,
        };
        let writer = self.begin_write(hint).await?;
        write_stream_and_commit(writer, stream).await
    }

    fn get(&self, key: &str) -> impl std::future::Future<Output = anyhow::Result<Bytes>> + Send {
        // Validate key to prevent path traversal
        let validation_result = validate_key(key);

        let path = self.base_dir.join(key);
        async move {
            validation_result?;
            let data = tokio::fs::read(path).await?;
            Ok(Bytes::from(data))
        }
    }

    fn get_stream(
        &self,
        key: &str,
    ) -> impl std::future::Future<Output = anyhow::Result<(u64, ByteReadStream)>> + Send {
        let validation_result = validate_key(key);
        let path = self.base_dir.join(key);
        async move {
            validation_result?;
            let file = tokio::fs::File::open(&path).await?;
            let len = file.metadata().await?.len();
            let stream = tokio_util::io::ReaderStream::new(file);
            Ok((len, Box::pin(stream) as ByteReadStream))
        }
    }

    fn delete(&self, key: &str) -> impl std::future::Future<Output = anyhow::Result<()>> + Send {
        // Validate key to prevent path traversal
        let validation_result = validate_key(key);

        let path = self.base_dir.join(key);
        async move {
            validation_result?;
            if path.exists() {
                tokio::fs::remove_file(path).await?;
            }
            Ok(())
        }
    }

    fn url_for(&self, key: &str) -> String {
        if let Some(base) = &self.url_base {
            format!("{}/{}", base.trim_end_matches('/'), key)
        } else {
            format!("file://{}", self.base_dir.join(key).to_string_lossy())
        }
    }

    fn get_as_path(
        &self,
        key: &str,
    ) -> impl std::future::Future<Output = anyhow::Result<(PathBuf, bool)>> + Send {
        // Validate key to prevent path traversal
        let validation_result = validate_key(key);
        let path = self.base_dir.join(key);

        async move {
            validation_result?;
            if !path.exists() {
                anyhow::bail!("File not found: {}", path.display());
            }
            // Filesystem: return actual path, is_temp=false (no cleanup needed)
            Ok((path, false))
        }
    }

    fn list_blobs(
        &self,
    ) -> impl std::future::Future<Output = anyhow::Result<Option<Vec<ListedBlob>>>> + Send {
        let base = self.base_dir.clone();
        async move {
            tokio::task::spawn_blocking(move || list_files(&base).map(Some))
                .await
                .map_err(|e| anyhow::anyhow!("listing blobs failed: {e}"))?
        }
    }

    async fn begin_write(&self, hint: WriteHint) -> anyhow::Result<Box<dyn BlobWriter>> {
        let key = Self::generate_key(hint.ext.as_deref(), hint.tenant_context.as_deref());
        let url = self.url_for(&key);
        let writer = writer::FsBlobWriter::open(&self.base_dir, key, url, &hint).await?;
        Ok(Box::new(writer))
    }

    fn get_range(
        &self,
        key: &str,
        range: Range<u64>,
    ) -> impl std::future::Future<Output = anyhow::Result<ByteReadStream>> + Send {
        let validation_result = validate_key(key);
        let path = self.base_dir.join(key);
        async move {
            use tokio::io::{AsyncReadExt, AsyncSeekExt};
            validation_result?;
            let mut file = tokio::fs::File::open(&path).await?;
            let size = file.metadata().await?.len();
            check_range(&range, size)?;
            file.seek(std::io::SeekFrom::Start(range.start)).await?;
            let limited = file.take(range.end - range.start);
            let stream = tokio_util::io::ReaderStream::with_capacity(limited, 256 * 1024);
            Ok(Box::pin(stream) as ByteReadStream)
        }
    }

    fn head(
        &self,
        key: &str,
    ) -> impl std::future::Future<Output = anyhow::Result<Option<BlobMeta>>> + Send {
        let validation_result = validate_key(key);
        let path = self.base_dir.join(key);
        async move {
            validation_result?;
            match tokio::fs::metadata(&path).await {
                Ok(meta) if meta.is_file() => Ok(Some(BlobMeta {
                    size: meta.len(),
                    modified: meta.modified().ok().map(DateTime::<Utc>::from),
                })),
                Ok(_) => Ok(None),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(e.into()),
            }
        }
    }

    fn available_space(
        &self,
    ) -> impl std::future::Future<Output = anyhow::Result<Option<StorageSpace>>> + Send {
        let base = self.base_dir.clone();
        async move {
            tokio::task::spawn_blocking(move || {
                // The store's directory may not exist yet; its volume is the
                // nearest existing ancestor's.
                space::volume_space(&base).map(Some).ok_or_else(|| {
                    anyhow::anyhow!("free-space query failed for {}", base.display())
                })
            })
            .await
            .map_err(|e| anyhow::anyhow!("free-space query failed: {e}"))?
        }
    }

    fn discard_partial(
        &self,
        partial_id: &str,
    ) -> impl std::future::Future<Output = anyhow::Result<bool>> + Send {
        let path = self
            .base_dir
            .join(PARTIAL_DIR)
            .join(writer::partial_file_name(Some(partial_id)));
        let stable = !partial_id.is_empty();
        async move {
            if !stable {
                return Ok(false);
            }
            match tokio::fs::remove_file(&path).await {
                Ok(()) => Ok(true),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
                Err(e) => Err(e.into()),
            }
        }
    }

    fn sweep_partials(
        &self,
        older_than: std::time::Duration,
    ) -> impl std::future::Future<Output = anyhow::Result<usize>> + Send {
        let base = self.base_dir.clone();
        async move {
            tokio::task::spawn_blocking(move || writer::sweep_fs_partials(&base, older_than))
                .await
                .map_err(|e| anyhow::anyhow!("partial sweep failed: {e}"))?
        }
    }
}

/// Drain `stream` into `writer` and commit it; abort on any error.
async fn write_stream_and_commit<S>(
    mut writer: Box<dyn BlobWriter>,
    stream: S,
) -> anyhow::Result<StoredObject>
where
    S: Stream<Item = Result<Bytes, std::io::Error>> + Send,
{
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;

    let mut stream = Box::pin(stream);
    let copied: anyhow::Result<()> = async {
        while let Some(chunk) = stream.next().await.transpose()? {
            writer.write_all(&chunk).await?;
        }
        Ok(())
    }
    .await;
    match copied {
        Ok(()) => writer.commit().await,
        Err(e) => {
            let _ = writer.abort().await;
            Err(e)
        }
    }
}

/// Every regular file under `base`, keyed by its `/`-separated relative path.
/// A missing directory holds nothing.
fn list_files(base: &Path) -> anyhow::Result<Vec<ListedBlob>> {
    let mut out = Vec::new();
    if !base.exists() {
        return Ok(out);
    }
    let mut dirs = vec![base.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            let path = entry.path();
            if file_type.is_dir() {
                // Uncommitted writes are not blobs yet.
                if dir == base && entry.file_name() == PARTIAL_DIR {
                    continue;
                }
                dirs.push(path);
            } else if file_type.is_file() {
                let Ok(rel) = path.strip_prefix(base) else {
                    continue;
                };
                let key = rel
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/");
                let meta = entry.metadata()?;
                out.push(ListedBlob {
                    key,
                    size: meta.len(),
                    modified: meta.modified().ok().map(DateTime::<Utc>::from),
                });
            }
        }
    }
    Ok(out)
}

#[cfg(feature = "s3")]
pub struct S3BinaryStorage {
    client: s3::Client,
    bucket: String,
    // If set, compose public URL as `${public_base}/${key}`; otherwise fallback to client signer URL
    public_base: Option<String>,
}

#[cfg(feature = "s3")]
impl S3BinaryStorage {
    pub async fn from_env() -> anyhow::Result<Self> {
        // Environment variable names aligned with 4myhoneybee repo
        let access_key = std::env::var("R2_ACCESS_KEY")?;
        let secret_key = std::env::var("R2_SECRET_KEY")?;
        let bucket = std::env::var("R2_BUCKET_NAME")?;
        let endpoint = std::env::var("R2_ENDPOINT").unwrap_or_else(|_| {
            "https://6ddf83a18b9273e54bfe14726624cd0e.r2.cloudflarestorage.com".to_string()
        });
        let public_base = std::env::var("R2_PUBLIC_BASE_URL").ok();

        let conf = s3::config::Builder::new()
            .region(s3::config::Region::new("auto"))
            .credentials_provider(s3::config::Credentials::new(
                access_key, secret_key, None, None, "user",
            ))
            .endpoint_url(endpoint)
            .behavior_version(s3::config::BehaviorVersion::latest())
            .build();

        let client = s3::Client::from_conf(conf);
        Ok(Self {
            client,
            bucket,
            public_base,
        })
    }
}
#[cfg(feature = "s3")]
impl BinaryStorage for S3BinaryStorage {
    fn put_stream<'a, S>(
        &'a self,
        stream: S,
        content_type: Option<&'a str>,
        ext: Option<&'a str>,
        original_name: Option<&'a str>,
        size_hint: Option<u64>,
        tenant_context: Option<&'a str>,
    ) -> impl std::future::Future<Output = anyhow::Result<StoredObject>> + Send
    where
        S: Stream<Item = Result<Bytes, std::io::Error>> + Send + 'a,
    {
        // One write path: the multipart writer (a single PutObject below one
        // part), so nothing is spooled to a local temp file any more.
        let hint = WriteHint {
            content_type: content_type.map(str::to_string),
            ext: ext.map(str::to_string),
            original_name: original_name.map(str::to_string),
            tenant_context: tenant_context.map(str::to_string),
            size_hint,
            partial_id: None,
        };
        async move {
            let writer = self.begin_write(hint).await?;
            write_stream_and_commit(writer, stream).await
        }
    }

    fn get(&self, key: &str) -> impl std::future::Future<Output = anyhow::Result<Bytes>> + Send {
        // Validate key to prevent path traversal
        let validation_result = validate_key(key);

        let bucket = self.bucket.clone();
        let client = self.client.clone();
        let key = key.to_string();
        async move {
            validation_result?;
            let response = client.get_object().bucket(&bucket).key(&key).send().await?;
            let data = response.body.collect().await?;
            Ok(data.into_bytes())
        }
    }

    fn get_stream(
        &self,
        key: &str,
    ) -> impl std::future::Future<Output = anyhow::Result<(u64, ByteReadStream)>> + Send {
        let validation_result = validate_key(key);
        let bucket = self.bucket.clone();
        let client = self.client.clone();
        let key = key.to_string();
        async move {
            validation_result?;
            let response = client.get_object().bucket(&bucket).key(&key).send().await?;
            let len = response.content_length().unwrap_or(0).max(0) as u64;
            // Stream the object body straight through — no `collect()`, so the
            // whole object is never buffered in process memory.
            let reader = response.body.into_async_read();
            let stream = tokio_util::io::ReaderStream::new(reader);
            Ok((len, Box::pin(stream) as ByteReadStream))
        }
    }

    fn delete(&self, key: &str) -> impl std::future::Future<Output = anyhow::Result<()>> + Send {
        // Validate key to prevent path traversal
        let validation_result = validate_key(key);

        let bucket = self.bucket.clone();
        let client = self.client.clone();
        let key = key.to_string();
        async move {
            validation_result?;
            client
                .delete_object()
                .bucket(&bucket)
                .key(&key)
                .send()
                .await?;
            Ok(())
        }
    }

    fn url_for(&self, key: &str) -> String {
        if let Some(base) = &self.public_base {
            format!("{}/{}", base.trim_end_matches('/'), key)
        } else {
            key.to_string()
        }
    }

    fn begin_write(
        &self,
        hint: WriteHint,
    ) -> impl std::future::Future<Output = anyhow::Result<Box<dyn BlobWriter>>> + Send {
        let key = FilesystemBinaryStorage::generate_key(
            hint.ext.as_deref(),
            hint.tenant_context.as_deref(),
        );
        let url = self.url_for(&key);
        let writer = s3_writer::S3BlobWriter::new(
            self.client.clone(),
            self.bucket.clone(),
            key,
            url,
            hint.original_name,
            hint.content_type,
        );
        async move { Ok(Box::new(writer) as Box<dyn BlobWriter>) }
    }

    fn get_range(
        &self,
        key: &str,
        range: Range<u64>,
    ) -> impl std::future::Future<Output = anyhow::Result<ByteReadStream>> + Send {
        let validation_result = validate_key(key);
        let bucket = self.bucket.clone();
        let client = self.client.clone();
        let key = key.to_string();
        async move {
            validation_result?;
            if range.start > range.end {
                anyhow::bail!("range {}..{} is inverted", range.start, range.end);
            }
            if range.start == range.end {
                let empty = futures_util::stream::empty::<std::io::Result<Bytes>>();
                return Ok(Box::pin(empty) as ByteReadStream);
            }
            let response = client
                .get_object()
                .bucket(&bucket)
                .key(&key)
                .range(format!("bytes={}-{}", range.start, range.end - 1))
                .send()
                .await?;
            let reader = response.body.into_async_read();
            let stream = tokio_util::io::ReaderStream::new(reader);
            Ok(Box::pin(stream) as ByteReadStream)
        }
    }

    fn head(
        &self,
        key: &str,
    ) -> impl std::future::Future<Output = anyhow::Result<Option<BlobMeta>>> + Send {
        let validation_result = validate_key(key);
        let bucket = self.bucket.clone();
        let client = self.client.clone();
        let key = key.to_string();
        async move {
            validation_result?;
            match client.head_object().bucket(&bucket).key(&key).send().await {
                Ok(out) => Ok(Some(BlobMeta {
                    size: out.content_length().unwrap_or(0).max(0) as u64,
                    modified: out
                        .last_modified()
                        .and_then(|t| DateTime::<Utc>::from_timestamp(t.secs(), t.subsec_nanos())),
                })),
                Err(e) => {
                    let not_found = e
                        .as_service_error()
                        .map(|se| se.is_not_found())
                        .unwrap_or(false);
                    if not_found {
                        Ok(None)
                    } else {
                        Err(e.into())
                    }
                }
            }
        }
    }

    fn sweep_partials(
        &self,
        older_than: std::time::Duration,
    ) -> impl std::future::Future<Output = anyhow::Result<usize>> + Send {
        let client = self.client.clone();
        let bucket = self.bucket.clone();
        async move { s3_writer::sweep_s3_multipart(&client, &bucket, older_than).await }
    }

    fn get_as_path(
        &self,
        key: &str,
    ) -> impl std::future::Future<Output = anyhow::Result<(PathBuf, bool)>> + Send {
        // Validate key to prevent path traversal
        let validation_result = validate_key(key);

        let bucket = self.bucket.clone();
        let client = self.client.clone();
        let key = key.to_string();

        async move {
            use tokio::io::AsyncWriteExt;

            validation_result?;

            // Download from S3
            let response = client.get_object().bucket(&bucket).key(&key).send().await?;
            let data = response.body.collect().await?;
            let bytes = data.into_bytes();

            // Write to temp file
            let temp_dir = std::env::temp_dir().join("raisin-pdf-processing");
            tokio::fs::create_dir_all(&temp_dir).await?;
            let temp_path = temp_dir.join(format!("{}.tmp", nanoid::nanoid!()));
            let mut file = tokio::fs::File::create(&temp_path).await?;
            file.write_all(&bytes).await?;
            file.flush().await?;

            // S3: return temp path, is_temp=true (caller must cleanup)
            Ok((temp_path, true))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_filesystem_store_lists_what_it_wrote() {
        let dir = tempfile::tempdir().unwrap();
        let store = FilesystemBinaryStorage::new(dir.path(), None);
        let a = store
            .put_bytes(b"abc", None, Some("rap"), None, Some("t1"))
            .await
            .unwrap();
        let b = store
            .put_bytes(b"de", None, Some("jpg"), None, None)
            .await
            .unwrap();

        let mut listed = store.list_blobs().await.unwrap().expect("filesystem lists");
        listed.sort_by(|x, y| x.key.cmp(&y.key));
        let mut expected = vec![(a.key, 3), (b.key, 2)];
        expected.sort();
        let got: Vec<(String, u64)> = listed.iter().map(|l| (l.key.clone(), l.size)).collect();
        assert_eq!(got, expected);
        assert!(listed.iter().all(|l| l.modified.is_some()));

        // A store whose directory was never created holds nothing.
        let empty = FilesystemBinaryStorage::new(dir.path().join("missing"), None);
        assert!(empty.list_blobs().await.unwrap().unwrap().is_empty());
    }

    async fn read_all(stream: ByteReadStream) -> Vec<u8> {
        use futures_util::StreamExt;
        let chunks: Vec<_> = stream.collect().await;
        chunks
            .into_iter()
            .flat_map(|c| c.unwrap().to_vec())
            .collect()
    }

    fn files_under(dir: &Path) -> usize {
        list_files(dir).map(|v| v.len()).unwrap_or(0)
            + std::fs::read_dir(dir.join(PARTIAL_DIR))
                .map(|d| d.count())
                .unwrap_or(0)
    }

    #[tokio::test]
    async fn an_aborted_write_leaves_no_object() {
        use tokio::io::AsyncWriteExt;
        let dir = tempfile::tempdir().unwrap();
        let store = FilesystemBinaryStorage::new(dir.path(), None);
        let mut w = store
            .begin_write(WriteHint::new().ext("rap").tenant(Some("t1")))
            .await
            .unwrap();
        w.write_all(&[7u8; 100_000]).await.unwrap();
        assert_eq!(w.bytes_written(), 100_000);
        let key = w.key().to_string();
        // Invisible until commit.
        assert!(store.head(&key).await.unwrap().is_none());
        w.abort().await.unwrap();
        assert!(store.head(&key).await.unwrap().is_none());
        assert_eq!(files_under(dir.path()), 0);
    }

    #[tokio::test]
    async fn a_dropped_write_leaves_no_object() {
        use tokio::io::AsyncWriteExt;
        let dir = tempfile::tempdir().unwrap();
        let store = FilesystemBinaryStorage::new(dir.path(), None);
        {
            let mut w = store.begin_write(WriteHint::new()).await.unwrap();
            w.write_all(b"half an archive").await.unwrap();
        }
        assert_eq!(files_under(dir.path()), 0);
    }

    #[tokio::test]
    async fn a_committed_write_reads_back_by_range() {
        use tokio::io::AsyncWriteExt;
        let dir = tempfile::tempdir().unwrap();
        let store = FilesystemBinaryStorage::new(dir.path(), None);
        let data: Vec<u8> = (0..1_000_000u32).map(|i| (i % 251) as u8).collect();
        let mut w = store
            .begin_write(
                WriteHint::new()
                    .ext("bin")
                    .content_type("application/octet-stream"),
            )
            .await
            .unwrap();
        for chunk in data.chunks(65_537) {
            w.write_all(chunk).await.unwrap();
        }
        let stored = w.commit().await.unwrap();
        assert_eq!(stored.size, data.len() as i64);
        assert_eq!(
            stored.mime_type.as_deref(),
            Some("application/octet-stream")
        );

        let meta = store.head(&stored.key).await.unwrap().unwrap();
        assert_eq!(meta.size, data.len() as u64);

        for r in [0..0u64, 0..1, 10..20, 999_990..1_000_000, 0..1_000_000] {
            let got = read_all(store.get_range(&stored.key, r.clone()).await.unwrap()).await;
            assert_eq!(got, &data[r.start as usize..r.end as usize]);
        }
        assert!(store.get_range(&stored.key, 0..1_000_001).await.is_err());
        // The partial directory is empty and is not listed as a blob.
        let listed = store.list_blobs().await.unwrap().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].key, stored.key);
    }

    #[tokio::test]
    async fn put_stream_goes_through_the_writer() {
        let dir = tempfile::tempdir().unwrap();
        let store = FilesystemBinaryStorage::new(dir.path(), None);
        let s = futures_util::stream::iter(vec![
            Ok(Bytes::from_static(b"ab")),
            Err(std::io::Error::other("client went away")),
        ]);
        assert!(store
            .put_stream(s, None, None, None, None, None)
            .await
            .is_err());
        // A failed upload leaves nothing, not a truncated blob.
        assert_eq!(files_under(dir.path()), 0);
    }

    #[tokio::test]
    async fn the_boot_sweep_removes_stale_partials_only() {
        use tokio::io::AsyncWriteExt;
        let dir = tempfile::tempdir().unwrap();
        let store = FilesystemBinaryStorage::new(dir.path(), None);
        // Simulate a process killed mid-write: a partial nobody owns.
        let mut w = store.begin_write(WriteHint::new()).await.unwrap();
        w.write_all(b"orphan").await.unwrap();
        std::mem::forget(w);
        assert_eq!(files_under(dir.path()), 1);
        // Fresh partials survive a sweep with a grace period...
        assert_eq!(
            store
                .sweep_partials(std::time::Duration::from_secs(3600))
                .await
                .unwrap(),
            0
        );
        // ...stale ones do not.
        assert_eq!(
            store
                .sweep_partials(std::time::Duration::ZERO)
                .await
                .unwrap(),
            1
        );
        assert_eq!(files_under(dir.path()), 0);
    }

    #[tokio::test]
    async fn free_space_is_observable_on_the_filesystem() {
        let dir = tempfile::tempdir().unwrap();
        let store = FilesystemBinaryStorage::new(dir.path().join("not-yet-created"), None);
        let space = store.available_space().await.unwrap().unwrap();
        assert!(space.total > 0);
        assert!(space.available <= space.total);
    }

    #[tokio::test]
    async fn the_dyn_view_streams_too() {
        use tokio::io::AsyncWriteExt;
        let dir = tempfile::tempdir().unwrap();
        let store: std::sync::Arc<dyn DynBinaryStorage> = erase(std::sync::Arc::new(
            FilesystemBinaryStorage::new(dir.path(), None),
        ));
        let mut w = store.begin_write(WriteHint::new()).await.unwrap();
        w.write_all(b"0123456789").await.unwrap();
        let stored = w.commit().await.unwrap();
        let got = read_all(store.get_range(&stored.key, 3..7).await.unwrap()).await;
        assert_eq!(got, b"3456");
        store.delete(&stored.key).await.unwrap();
        assert!(store.head(&stored.key).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn a_stable_partial_id_is_reused_and_discardable() {
        use tokio::io::AsyncWriteExt;
        let dir = tempfile::tempdir().unwrap();
        let store = FilesystemBinaryStorage::new(dir.path(), None);
        // An interrupted attempt…
        let mut w = store
            .begin_write(WriteHint::new().partial_id("pkgbuild-job/1"))
            .await
            .unwrap();
        w.write_all(b"first attempt, never committed")
            .await
            .unwrap();
        std::mem::forget(w);
        let part = dir.path().join(PARTIAL_DIR).join("pkgbuild-job1.part");
        assert!(part.exists());
        // …is found by name and discarded.
        assert!(store.discard_partial("pkgbuild-job/1").await.unwrap());
        assert!(!part.exists());
        assert!(!store.discard_partial("pkgbuild-job/1").await.unwrap());
        // A retry under the same id starts from an empty file.
        let mut w = store
            .begin_write(WriteHint::new().partial_id("pkgbuild-job1"))
            .await
            .unwrap();
        w.write_all(b"ok").await.unwrap();
        let stored = w.commit().await.unwrap();
        assert_eq!(store.get(&stored.key).await.unwrap().as_ref(), b"ok");
    }
}
