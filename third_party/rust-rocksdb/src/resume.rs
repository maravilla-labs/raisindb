//! `DB::resume()`: clear a stopped database's background error.
//!
//! Not part of upstream rust-rocksdb; see the vendor note in Cargo.toml.

use std::ffi::CStr;

use crate::db::{DBCommon, DBInner};
use crate::{Error, ThreadMode, ffi};

unsafe extern "C" {
    fn raisin_rocksdb_resume(db: *mut ffi::rocksdb_t) -> *mut libc::c_char;
}

impl<T: ThreadMode, D: DBInner> DBCommon<T, D> {
    /// Call `rocksdb::DB::Resume()`.
    ///
    /// After a hard background error (a failed WAL write, e.g. on a full
    /// disk) RocksDB stops accepting writes. It retries once on its own, and
    /// if that retry fails too it gives up until something calls Resume.
    /// On a database that is not stopped this is a cheap no-op returning Ok.
    pub fn resume(&self) -> Result<(), Error> {
        let err = unsafe { raisin_rocksdb_resume(self.inner.inner()) };
        if err.is_null() {
            return Ok(());
        }
        let message = unsafe { CStr::from_ptr(err) }.to_string_lossy().into_owned();
        unsafe { libc::free(err.cast()) };
        Err(Error::new(message))
    }
}
