#[cfg(feature = "s3")]
use aws_sdk_s3 as s3;
#[cfg(feature = "s3")]
use aws_sdk_s3::primitives::ByteStream;
use bytes::Bytes;
use chrono::{DateTime, Utc};
use futures_core::Stream;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

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
        _size_hint: Option<u64>,
        tenant_context: Option<&'a str>,
    ) -> anyhow::Result<StoredObject>
    where
        S: Stream<Item = Result<Bytes, std::io::Error>> + Send + 'a,
    {
        use futures_util::StreamExt;
        use tokio::io::AsyncWriteExt;

        // Generate key with optional tenant prefix for multi-tenant isolation
        let key = Self::generate_key(ext, tenant_context);
        let path = self.base_dir.join(&key);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = tokio::fs::File::create(&path).await?;
        let mut total: i64 = 0;
        let mut stream = Box::pin(stream);
        while let Some(chunk) = stream.next().await.transpose()? {
            total += chunk.len() as i64;
            file.write_all(&chunk).await?;
        }
        file.flush().await?;
        let url = if let Some(base) = &self.url_base {
            format!("{}/{}", base.trim_end_matches('/'), key)
        } else {
            format!("file://{}", path.to_string_lossy())
        };
        let now = Utc::now();
        Ok(StoredObject {
            key,
            url,
            name: original_name.map(|s| s.to_string()),
            size: total,
            mime_type: content_type.map(|s| s.to_string()),
            created_at: now,
            updated_at: now,
        })
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
        _size_hint: Option<u64>,
        tenant_context: Option<&'a str>,
    ) -> impl std::future::Future<Output = anyhow::Result<StoredObject>> + Send
    where
        S: Stream<Item = Result<Bytes, std::io::Error>> + Send + 'a,
    {
        async move {
            use futures_util::StreamExt;
            use tokio::io::AsyncWriteExt;
            // Spool to a temporary file to avoid full buffering in memory and satisfy SDK body bounds
            let tmp_path =
                std::env::temp_dir().join(format!("raisin-upload-{}.tmp", nanoid::nanoid!()));
            let mut file = tokio::fs::File::create(&tmp_path).await?;
            let mut total: i64 = 0;
            let mut s = Box::pin(stream);
            while let Some(chunk) = s.next().await.transpose()? {
                total += chunk.len() as i64;
                file.write_all(&chunk).await?;
            }
            file.flush().await?;

            // Generate key with optional tenant prefix for multi-tenant isolation
            let key = FilesystemBinaryStorage::generate_key(ext, tenant_context);
            let body = ByteStream::from_path(tmp_path.clone()).await?;
            self.client
                .put_object()
                .bucket(&self.bucket)
                .key(&key)
                .set_content_type(content_type.map(|s| s.to_string()))
                .body(body)
                .send()
                .await?;
            let _ = tokio::fs::remove_file(tmp_path).await;
            let now = Utc::now();
            let url = if let Some(base) = &self.public_base {
                format!("{}/{}", base.trim_end_matches('/'), key)
            } else {
                key.clone()
            };
            Ok(StoredObject {
                key,
                url,
                name: original_name.map(|s| s.to_string()),
                size: total,
                mime_type: content_type.map(|s| s.to_string()),
                created_at: now,
                updated_at: now,
            })
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
