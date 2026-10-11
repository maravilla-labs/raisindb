//! An object-safe view of [`BinaryStorage`].
//!
//! [`BinaryStorage`] returns `impl Future`, which keeps its many generic call
//! sites zero-cost but makes `dyn BinaryStorage` impossible. Code that is
//! wired once at startup and must not grow a type parameter — the package
//! build job — holds an `Arc<dyn DynBinaryStorage>` instead, made with
//! [`erase`]. (A wrapper rather than a blanket impl, so calling a method on a
//! concrete store is never ambiguous between the two traits.)

use crate::writer::{BlobMeta, BlobWriter, BoxFuture, StorageSpace, WriteHint};
use crate::{BinaryStorage, ByteReadStream};
use std::ops::Range;
use std::sync::Arc;

/// Type-erase a store for code that cannot be generic over it.
pub fn erase<T: BinaryStorage + 'static>(store: Arc<T>) -> Arc<dyn DynBinaryStorage> {
    Arc::new(Erased(store))
}

struct Erased<T>(Arc<T>);

/// The streaming subset of [`BinaryStorage`], object safe.
pub trait DynBinaryStorage: Send + Sync {
    fn begin_write(&self, hint: WriteHint) -> BoxFuture<'_, anyhow::Result<Box<dyn BlobWriter>>>;

    fn get_stream<'a>(
        &'a self,
        key: &'a str,
    ) -> BoxFuture<'a, anyhow::Result<(u64, ByteReadStream)>>;

    fn get_range<'a>(
        &'a self,
        key: &'a str,
        range: Range<u64>,
    ) -> BoxFuture<'a, anyhow::Result<ByteReadStream>>;

    fn head<'a>(&'a self, key: &'a str) -> BoxFuture<'a, anyhow::Result<Option<BlobMeta>>>;

    fn delete<'a>(&'a self, key: &'a str) -> BoxFuture<'a, anyhow::Result<()>>;

    fn available_space(&self) -> BoxFuture<'_, anyhow::Result<Option<StorageSpace>>>;

    fn sweep_partials(
        &self,
        older_than: std::time::Duration,
    ) -> BoxFuture<'_, anyhow::Result<usize>>;
}

impl<T: BinaryStorage + 'static> DynBinaryStorage for Erased<T> {
    fn begin_write(&self, hint: WriteHint) -> BoxFuture<'_, anyhow::Result<Box<dyn BlobWriter>>> {
        Box::pin(BinaryStorage::begin_write(&*self.0, hint))
    }

    fn get_stream<'a>(
        &'a self,
        key: &'a str,
    ) -> BoxFuture<'a, anyhow::Result<(u64, ByteReadStream)>> {
        Box::pin(BinaryStorage::get_stream(&*self.0, key))
    }

    fn get_range<'a>(
        &'a self,
        key: &'a str,
        range: Range<u64>,
    ) -> BoxFuture<'a, anyhow::Result<ByteReadStream>> {
        Box::pin(BinaryStorage::get_range(&*self.0, key, range))
    }

    fn head<'a>(&'a self, key: &'a str) -> BoxFuture<'a, anyhow::Result<Option<BlobMeta>>> {
        Box::pin(BinaryStorage::head(&*self.0, key))
    }

    fn delete<'a>(&'a self, key: &'a str) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(BinaryStorage::delete(&*self.0, key))
    }

    fn available_space(&self) -> BoxFuture<'_, anyhow::Result<Option<StorageSpace>>> {
        Box::pin(BinaryStorage::available_space(&*self.0))
    }

    fn sweep_partials(
        &self,
        older_than: std::time::Duration,
    ) -> BoxFuture<'_, anyhow::Result<usize>> {
        Box::pin(BinaryStorage::sweep_partials(&*self.0, older_than))
    }
}
