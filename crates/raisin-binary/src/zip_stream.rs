//! Streaming ZIP64 archives: written without seeking, read with ranged reads.
//!
//! # Writing
//!
//! [`ZipStreamWriter`] appends entries to any [`AsyncWrite`] — a
//! [`BlobWriter`](crate::BlobWriter), typically — and never seeks back. Each
//! entry is a local header with general-purpose bit 3 set (sizes and CRC in a
//! trailing *data descriptor*), its body (stored or raw-deflated), then the
//! descriptor. Local headers always carry a ZIP64 extra field, so descriptors
//! always use 8-byte sizes and an entry may be any size. The central
//! directory and, when anything overflows 32 bits, the ZIP64 end records are
//! written by [`ZipStreamWriter::finish`]. Readers that use the central
//! directory — `zip::ZipArchive`, `unzip`, `bsdtar`, Python's `zipfile` —
//! read the result normally.
//!
//! Central-directory records are spilled to a file (when a spill directory is
//! given) instead of kept in memory, so a million-entry archive costs a
//! constant amount of memory to write.
//!
//! Every entry body is also SHA-256'd as it is written (over the uncompressed
//! bytes), which is what a package inventory records per entry.
//!
//! # Reading
//!
//! [`read_central_directory`] locates and parses the central directory with
//! two or three ranged reads from the end of the archive, and
//! [`entry_data_range`] finds one entry's body with one more. Nothing reads
//! the archive front to back.

use crate::hashing::StreamHasher;
use bytes::Bytes;
use chrono::{DateTime, Datelike, Timelike, Utc};
use std::future::Future;
use std::ops::Range;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncWrite, AsyncWriteExt};

const LOCAL_HEADER_SIG: u32 = 0x0403_4b50;
const DATA_DESCRIPTOR_SIG: u32 = 0x0807_4b50;
const CENTRAL_HEADER_SIG: u32 = 0x0201_4b50;
const EOCD_SIG: u32 = 0x0605_4b50;
const ZIP64_EOCD_SIG: u32 = 0x0606_4b50;
const ZIP64_LOCATOR_SIG: u32 = 0x0706_4b50;
const ZIP64_EXTRA_TAG: u16 = 0x0001;

/// "Version needed to extract": 4.5, ZIP64.
const VERSION_ZIP64: u16 = 45;
/// "Version made by": Unix, 4.5 — so external attributes carry Unix modes.
const VERSION_MADE_BY: u16 = (3 << 8) | VERSION_ZIP64;
/// Bit 3 (sizes in data descriptor) + bit 11 (UTF-8 names).
const FLAGS: u16 = 0x0008 | 0x0800;

const METHOD_STORED: u16 = 0;
const METHOD_DEFLATED: u16 = 8;

const U32_MAX: u64 = 0xFFFF_FFFF;
const U16_MAX: u64 = 0xFFFF;

/// How an entry's body is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    /// Stored as-is. Right for media and anything already compressed.
    Stored,
    /// Raw deflate at the given level (0–9).
    Deflate(u32),
}

impl Compression {
    /// Deflate level 6 for text-like content, stored for content types that
    /// are already compressed (deflating them burns CPU for ~0 %).
    pub fn for_content_type(content_type: Option<&str>) -> Self {
        match content_type {
            Some(ct) if is_precompressed(ct) => Compression::Stored,
            _ => Compression::Deflate(6),
        }
    }

    fn method(self) -> u16 {
        match self {
            Compression::Stored => METHOD_STORED,
            Compression::Deflate(_) => METHOD_DEFLATED,
        }
    }
}

fn is_precompressed(ct: &str) -> bool {
    let ct = ct.to_ascii_lowercase();
    if ct.starts_with("image/svg") {
        return false;
    }
    ct.starts_with("image/")
        || ct.starts_with("video/")
        || ct.starts_with("audio/")
        || matches!(
            ct.as_str(),
            "application/zip"
                | "application/gzip"
                | "application/x-gzip"
                | "application/pdf"
                | "application/x-7z-compressed"
                | "application/x-rar-compressed"
                | "application/zstd"
                | "application/x-xz"
                | "application/x-bzip2"
                | "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
                | "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
                | "application/vnd.openxmlformats-officedocument.presentationml.presentation"
                | "font/woff"
                | "font/woff2"
        )
}

/// What one finished entry was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryDigest {
    /// Lower-case hex SHA-256 of the uncompressed body.
    pub sha256: String,
    /// Uncompressed size.
    pub size: u64,
    /// Size as stored in the archive.
    pub compressed_size: u64,
    pub crc32: u32,
}

/// Where central-directory records accumulate until [`ZipStreamWriter::finish`].
enum CentralSpill {
    Memory(Vec<u8>),
    File {
        path: SpillPath,
        writer: tokio::io::BufWriter<tokio::fs::File>,
    },
}

/// Removes the spill file when dropped, whatever happened to the writer.
struct SpillPath(PathBuf);

impl Drop for SpillPath {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

impl CentralSpill {
    async fn append(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        match self {
            CentralSpill::Memory(v) => {
                v.extend_from_slice(bytes);
                Ok(())
            }
            CentralSpill::File { writer, .. } => writer.write_all(bytes).await,
        }
    }
}

/// A streaming ZIP64 writer. See the [module docs](self).
pub struct ZipStreamWriter<W> {
    out: W,
    offset: u64,
    entries: u64,
    central: Option<CentralSpill>,
    dos_time: (u16, u16),
    force_zip64: bool,
}

impl<W: AsyncWrite + Unpin + Send> ZipStreamWriter<W> {
    /// A writer whose central directory is kept in memory. Fine for small
    /// archives and tests; use [`with_spill`](Self::with_spill) for large ones.
    pub fn new(out: W) -> Self {
        Self {
            out,
            offset: 0,
            entries: 0,
            central: Some(CentralSpill::Memory(Vec::new())),
            dos_time: dos_datetime(Utc::now()),
            force_zip64: false,
        }
    }

    /// A writer that spills its central directory to a file in `spill_dir`,
    /// so memory stays constant in the number of entries. The spill file is
    /// removed by `finish` and when the writer is dropped.
    pub async fn with_spill(out: W, spill_dir: &Path) -> std::io::Result<Self> {
        tokio::fs::create_dir_all(spill_dir).await?;
        let path = spill_dir.join(format!("{}.zipcd", nanoid::nanoid!()));
        let file = tokio::fs::File::create(&path).await?;
        let mut w = Self::new(out);
        w.central = Some(CentralSpill::File {
            path: SpillPath(path),
            writer: tokio::io::BufWriter::with_capacity(256 * 1024, file),
        });
        Ok(w)
    }

    /// Timestamp written on every entry (default: when the writer was made).
    pub fn set_modified(&mut self, at: DateTime<Utc>) {
        self.dos_time = dos_datetime(at);
    }

    /// Write ZIP64 records even when nothing overflows — so tests can
    /// exercise the large-archive layout with small data.
    pub fn force_zip64(&mut self, on: bool) {
        self.force_zip64 = on;
    }

    /// Bytes written to the output so far.
    pub fn bytes_written(&self) -> u64 {
        self.offset
    }

    pub fn entries(&self) -> u64 {
        self.entries
    }

    pub fn get_ref(&self) -> &W {
        &self.out
    }

    async fn put(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.out.write_all(bytes).await?;
        self.offset += bytes.len() as u64;
        Ok(())
    }

    /// Begin an entry. Write its body with [`EntryWriter::write`] and end it
    /// with [`EntryWriter::finish`] before starting the next one.
    pub async fn start_entry(
        &mut self,
        name: &str,
        compression: Compression,
    ) -> std::io::Result<EntryWriter<'_, W>> {
        let name = name.trim_start_matches('/');
        if name.is_empty() || name.len() > u16::MAX as usize {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "zip entry name must be 1..=65535 bytes",
            ));
        }
        let header_offset = self.offset;
        let (time, date) = self.dos_time;
        let mut h = Vec::with_capacity(30 + name.len() + 20);
        put_u32(&mut h, LOCAL_HEADER_SIG);
        put_u16(&mut h, VERSION_ZIP64);
        put_u16(&mut h, FLAGS);
        put_u16(&mut h, compression.method());
        put_u16(&mut h, time);
        put_u16(&mut h, date);
        put_u32(&mut h, 0); // crc: in the descriptor
        put_u32(&mut h, U32_MAX as u32); // sizes: in the ZIP64 extra / descriptor
        put_u32(&mut h, U32_MAX as u32);
        put_u16(&mut h, name.len() as u16);
        put_u16(&mut h, 20);
        h.extend_from_slice(name.as_bytes());
        put_u16(&mut h, ZIP64_EXTRA_TAG);
        put_u16(&mut h, 16);
        put_u64(&mut h, 0);
        put_u64(&mut h, 0);
        self.put(&h).await?;

        let deflater = match compression {
            Compression::Stored => None,
            Compression::Deflate(level) => Some(flate2::Compress::new(
                flate2::Compression::new(level.min(9)),
                false,
            )),
        };
        let data_start = self.offset;
        Ok(EntryWriter {
            zip: self,
            name: name.to_string(),
            compression,
            header_offset,
            data_start,
            crc: crc32fast::Hasher::new(),
            hasher: StreamHasher::new(),
            deflater,
            scratch: Vec::new(),
        })
    }

    /// Write a whole entry from one buffer.
    pub async fn write_entry(
        &mut self,
        name: &str,
        compression: Compression,
        data: &[u8],
    ) -> std::io::Result<EntryDigest> {
        let mut e = self.start_entry(name, compression).await?;
        e.write(data).await?;
        e.finish().await
    }

    /// Write the central directory and end records, flush, and hand back
    /// the output (still uncommitted, if it is a blob writer).
    pub async fn finish(mut self) -> std::io::Result<W> {
        let central = self.central.take();
        let cd_start = self.offset;
        match central {
            Some(CentralSpill::Memory(v)) => self.put(&v).await?,
            Some(CentralSpill::File { path, mut writer }) => {
                writer.flush().await?;
                drop(writer);
                let copied = async {
                    let mut file = tokio::fs::File::open(&path.0).await?;
                    let n = tokio::io::copy(&mut file, &mut self.out).await?;
                    self.offset += n;
                    std::io::Result::Ok(())
                }
                .await;
                drop(path);
                copied?;
            }
            None => {}
        }
        let cd_size = self.offset - cd_start;
        let needs_zip64 = self.force_zip64
            || self.entries >= U16_MAX
            || cd_start >= U32_MAX
            || cd_size >= U32_MAX;

        let mut tail = Vec::with_capacity(56 + 20 + 22);
        if needs_zip64 {
            let zip64_eocd_offset = self.offset;
            put_u32(&mut tail, ZIP64_EOCD_SIG);
            put_u64(&mut tail, 44); // size of the remaining record
            put_u16(&mut tail, VERSION_MADE_BY);
            put_u16(&mut tail, VERSION_ZIP64);
            put_u32(&mut tail, 0); // this disk
            put_u32(&mut tail, 0); // disk with the central directory
            put_u64(&mut tail, self.entries);
            put_u64(&mut tail, self.entries);
            put_u64(&mut tail, cd_size);
            put_u64(&mut tail, cd_start);

            put_u32(&mut tail, ZIP64_LOCATOR_SIG);
            put_u32(&mut tail, 0);
            put_u64(&mut tail, zip64_eocd_offset);
            put_u32(&mut tail, 1);
        }
        put_u32(&mut tail, EOCD_SIG);
        put_u16(&mut tail, 0);
        put_u16(&mut tail, 0);
        let entries16 = if needs_zip64 { U16_MAX } else { self.entries } as u16;
        put_u16(&mut tail, entries16);
        put_u16(&mut tail, entries16);
        put_u32(
            &mut tail,
            if needs_zip64 { U32_MAX } else { cd_size } as u32,
        );
        put_u32(
            &mut tail,
            if needs_zip64 { U32_MAX } else { cd_start } as u32,
        );
        put_u16(&mut tail, 0); // comment length
        self.put(&tail).await?;
        self.out.flush().await?;
        Ok(self.out)
    }
}

/// One open entry of a [`ZipStreamWriter`].
pub struct EntryWriter<'a, W: AsyncWrite + Unpin + Send> {
    zip: &'a mut ZipStreamWriter<W>,
    name: String,
    compression: Compression,
    header_offset: u64,
    data_start: u64,
    crc: crc32fast::Hasher,
    hasher: StreamHasher,
    deflater: Option<flate2::Compress>,
    scratch: Vec<u8>,
}

impl<W: AsyncWrite + Unpin + Send> EntryWriter<'_, W> {
    /// Uncompressed bytes accepted so far.
    pub fn size(&self) -> u64 {
        self.hasher.size()
    }

    pub async fn write(&mut self, data: &[u8]) -> std::io::Result<()> {
        if data.is_empty() {
            return Ok(());
        }
        self.crc.update(data);
        self.hasher.update(data);
        match self.deflater.as_mut() {
            None => self.zip.put(data).await,
            Some(_) => self.deflate(data, flate2::FlushCompress::None).await,
        }
    }

    async fn deflate(
        &mut self,
        mut input: &[u8],
        flush: flate2::FlushCompress,
    ) -> std::io::Result<()> {
        let finishing = matches!(flush, flate2::FlushCompress::Finish);
        loop {
            let deflater = self.deflater.as_mut().expect("deflate entry");
            self.scratch.clear();
            self.scratch.reserve(64 * 1024);
            let before_in = deflater.total_in();
            let status = deflater
                .compress_vec(input, &mut self.scratch, flush)
                .map_err(std::io::Error::other)?;
            let consumed = (deflater.total_in() - before_in) as usize;
            input = &input[consumed..];
            if !self.scratch.is_empty() {
                let chunk = std::mem::take(&mut self.scratch);
                self.zip.put(&chunk).await?;
                self.scratch = chunk;
            }
            let done = if finishing {
                status == flate2::Status::StreamEnd
            } else {
                input.is_empty() && self.scratch.len() < self.scratch.capacity()
            };
            if done {
                return Ok(());
            }
        }
    }

    /// End the entry: flush the compressor, write the data descriptor and
    /// record the central-directory entry.
    pub async fn finish(mut self) -> std::io::Result<EntryDigest> {
        if self.deflater.is_some() {
            self.deflate(&[], flate2::FlushCompress::Finish).await?;
        }
        let compressed_size = self.zip.offset - self.data_start;
        let size = self.hasher.size();
        let crc32 = self.crc.clone().finalize();

        let mut d = Vec::with_capacity(24);
        put_u32(&mut d, DATA_DESCRIPTOR_SIG);
        put_u32(&mut d, crc32);
        put_u64(&mut d, compressed_size);
        put_u64(&mut d, size);
        self.zip.put(&d).await?;

        let force = self.zip.force_zip64;
        let big_size = force || size >= U32_MAX;
        let big_compressed = force || compressed_size >= U32_MAX;
        let big_offset = force || self.header_offset >= U32_MAX;
        let mut extra = Vec::new();
        if big_size || big_compressed || big_offset {
            let fields = [big_size, big_compressed, big_offset]
                .iter()
                .filter(|b| **b)
                .count() as u16;
            put_u16(&mut extra, ZIP64_EXTRA_TAG);
            put_u16(&mut extra, fields * 8);
            if big_size {
                put_u64(&mut extra, size);
            }
            if big_compressed {
                put_u64(&mut extra, compressed_size);
            }
            if big_offset {
                put_u64(&mut extra, self.header_offset);
            }
        }
        let (time, date) = self.zip.dos_time;
        let mut c = Vec::with_capacity(46 + self.name.len() + extra.len());
        put_u32(&mut c, CENTRAL_HEADER_SIG);
        put_u16(&mut c, VERSION_MADE_BY);
        put_u16(&mut c, VERSION_ZIP64);
        put_u16(&mut c, FLAGS);
        put_u16(&mut c, self.compression.method());
        put_u16(&mut c, time);
        put_u16(&mut c, date);
        put_u32(&mut c, crc32);
        put_u32(
            &mut c,
            if big_compressed {
                U32_MAX
            } else {
                compressed_size
            } as u32,
        );
        put_u32(&mut c, if big_size { U32_MAX } else { size } as u32);
        put_u16(&mut c, self.name.len() as u16);
        put_u16(&mut c, extra.len() as u16);
        put_u16(&mut c, 0); // comment
        put_u16(&mut c, 0); // disk start
        put_u16(&mut c, 0); // internal attributes
        put_u32(&mut c, 0o100644 << 16); // regular file, rw-r--r--
        put_u32(
            &mut c,
            if big_offset {
                U32_MAX
            } else {
                self.header_offset
            } as u32,
        );
        c.extend_from_slice(self.name.as_bytes());
        c.extend_from_slice(&extra);
        self.zip
            .central
            .as_mut()
            .expect("central directory open until finish")
            .append(&c)
            .await?;
        self.zip.entries += 1;

        Ok(EntryDigest {
            sha256: std::mem::take(&mut self.hasher).finish().sha256,
            size,
            compressed_size,
            crc32,
        })
    }
}

fn dos_datetime(at: DateTime<Utc>) -> (u16, u16) {
    let year = at.year().clamp(1980, 2107) as u16;
    let time = ((at.hour() as u16) << 11) | ((at.minute() as u16) << 5) | (at.second() as u16 / 2);
    let date = ((year - 1980) << 9) | ((at.month() as u16) << 5) | at.day() as u16;
    (time, date)
}

fn put_u16(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_le_bytes());
}
fn put_u32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_le_bytes());
}
fn put_u64(v: &mut Vec<u8>, x: u64) {
    v.extend_from_slice(&x.to_le_bytes());
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

/// One central-directory entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CentralEntry {
    pub name: String,
    pub method: u16,
    pub crc32: u32,
    pub compressed_size: u64,
    pub size: u64,
    pub local_header_offset: u64,
}

impl CentralEntry {
    pub fn is_dir(&self) -> bool {
        self.name.ends_with('/')
    }
    pub fn is_deflated(&self) -> bool {
        self.method == METHOD_DEFLATED
    }
}

fn bad(msg: impl Into<String>) -> anyhow::Error {
    anyhow::anyhow!("not a readable zip archive: {}", msg.into())
}

struct Cursor<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn new(b: &'a [u8]) -> Self {
        Self { b, at: 0 }
    }
    fn take(&mut self, n: usize) -> anyhow::Result<&'a [u8]> {
        let s = self
            .b
            .get(self.at..self.at + n)
            .ok_or_else(|| bad("truncated record"))?;
        self.at += n;
        Ok(s)
    }
    fn u16(&mut self) -> anyhow::Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> anyhow::Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> anyhow::Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
}

/// Where the central directory is: `(entries, offset, size)`, from the end
/// records (one ranged read of the tail, one more for a ZIP64 record).
async fn locate_central_directory<F, Fut>(
    len: u64,
    fetch: &mut F,
) -> anyhow::Result<(u64, u64, u64)>
where
    F: FnMut(Range<u64>) -> Fut,
    Fut: Future<Output = anyhow::Result<Bytes>>,
{
    if len < 22 {
        return Err(bad("shorter than an end record"));
    }
    // EOCD (22 bytes) + max comment (65535) + room for the ZIP64 locator.
    let tail_len = len.min(22 + 65_535 + 20);
    let tail_start = len - tail_len;
    let tail = fetch(tail_start..len).await?;
    let eocd_pos = (0..=tail.len().saturating_sub(22))
        .rev()
        .find(|&i| tail[i..i + 4] == EOCD_SIG.to_le_bytes())
        .ok_or_else(|| bad("no end-of-central-directory record"))?;
    let mut c = Cursor::new(&tail[eocd_pos..]);
    c.take(4)?;
    c.u16()?;
    c.u16()?;
    c.u16()?;
    let mut entries = c.u16()? as u64;
    let mut cd_size = c.u32()? as u64;
    let mut cd_offset = c.u32()? as u64;

    // A ZIP64 locator sits immediately before the EOCD.
    if eocd_pos >= 20 && tail[eocd_pos - 20..eocd_pos - 16] == ZIP64_LOCATOR_SIG.to_le_bytes() {
        let mut l = Cursor::new(&tail[eocd_pos - 20..eocd_pos]);
        l.take(8)?;
        let zip64_eocd = l.u64()?;
        if zip64_eocd + 56 > len {
            return Err(bad("ZIP64 end record out of range"));
        }
        let rec = fetch(zip64_eocd..zip64_eocd + 56).await?;
        let mut r = Cursor::new(&rec);
        if r.u32()? != ZIP64_EOCD_SIG {
            return Err(bad("bad ZIP64 end record signature"));
        }
        r.take(8 + 2 + 2 + 4 + 4 + 8)?;
        entries = r.u64()?;
        cd_size = r.u64()?;
        cd_offset = r.u64()?;
    }
    if cd_offset.checked_add(cd_size).map_or(true, |end| end > len) {
        return Err(bad("central directory out of range"));
    }
    Ok((entries, cd_offset, cd_size))
}

/// Total length of the central-directory record starting at `b[0]`, once
/// its fixed part is available.
fn central_record_len(b: &[u8]) -> anyhow::Result<usize> {
    let mut c = Cursor::new(b);
    if c.u32()? != CENTRAL_HEADER_SIG {
        return Err(bad("bad central header signature"));
    }
    c.take(24)?;
    let name_len = c.u16()? as usize;
    let extra_len = c.u16()? as usize;
    let comment_len = c.u16()? as usize;
    Ok(46 + name_len + extra_len + comment_len)
}

/// Parse one central-directory record (which must be complete).
fn parse_central_record(c: &mut Cursor<'_>) -> anyhow::Result<CentralEntry> {
    if c.u32()? != CENTRAL_HEADER_SIG {
        return Err(bad("bad central header signature"));
    }
    c.take(2 + 2 + 2)?;
    let method = c.u16()?;
    c.take(4)?;
    let crc32 = c.u32()?;
    let mut compressed_size = c.u32()? as u64;
    let mut size = c.u32()? as u64;
    let name_len = c.u16()? as usize;
    let extra_len = c.u16()? as usize;
    let comment_len = c.u16()? as usize;
    c.take(2 + 2 + 4)?;
    let mut local_header_offset = c.u32()? as u64;
    let name = String::from_utf8_lossy(c.take(name_len)?).into_owned();
    let extra = c.take(extra_len)?;
    c.take(comment_len)?;

    let mut e = Cursor::new(extra);
    while e.at + 4 <= extra.len() {
        let tag = e.u16()?;
        let n = e.u16()? as usize;
        let body = e.take(n)?;
        if tag == ZIP64_EXTRA_TAG {
            let mut z = Cursor::new(body);
            if size == U32_MAX {
                size = z.u64()?;
            }
            if compressed_size == U32_MAX {
                compressed_size = z.u64()?;
            }
            if local_header_offset == U32_MAX {
                local_header_offset = z.u64()?;
            }
        }
    }
    Ok(CentralEntry {
        name,
        method,
        crc32,
        compressed_size,
        size,
        local_header_offset,
    })
}

/// Locate and parse the central directory of an archive of `len` bytes,
/// fetching only the ranges it needs through `fetch`. Refuses a central
/// directory larger than `max_cd_bytes`, which bounds the memory used.
pub async fn read_central_directory<F, Fut>(
    len: u64,
    mut fetch: F,
    max_cd_bytes: u64,
) -> anyhow::Result<Vec<CentralEntry>>
where
    F: FnMut(Range<u64>) -> Fut,
    Fut: Future<Output = anyhow::Result<Bytes>>,
{
    let (entries, cd_offset, cd_size) = locate_central_directory(len, &mut fetch).await?;
    if cd_size > max_cd_bytes {
        return Err(anyhow::anyhow!(
            "central directory is {} bytes, more than the {} allowed",
            cd_size,
            max_cd_bytes
        ));
    }
    let cd = fetch(cd_offset..cd_offset + cd_size).await?;
    let mut c = Cursor::new(&cd);
    let mut out = Vec::with_capacity(entries.min(1 << 20) as usize);
    for _ in 0..entries {
        out.push(parse_central_record(&mut c)?);
    }
    Ok(out)
}

/// Window used by [`find_entry`] to walk a central directory.
const FIND_WINDOW: u64 = 256 * 1024;

/// Find one entry by name, walking the central directory in fixed windows:
/// memory stays one window however many entries the archive has, which is
/// what lets a manifest be read out of a multi-gigabyte, million-entry
/// archive cheaply.
pub async fn find_entry<F, Fut>(
    len: u64,
    mut fetch: F,
    name: &str,
) -> anyhow::Result<Option<CentralEntry>>
where
    F: FnMut(Range<u64>) -> Fut,
    Fut: Future<Output = anyhow::Result<Bytes>>,
{
    let (entries, cd_offset, cd_size) = locate_central_directory(len, &mut fetch).await?;
    let cd_end = cd_offset + cd_size;
    let mut buf: Vec<u8> = Vec::new();
    let mut pos = 0usize;
    let mut next = cd_offset;
    for _ in 0..entries {
        // Make at least `n` bytes available from `pos`.
        macro_rules! ensure {
            ($n:expr) => {{
                let n: usize = $n;
                while buf.len() - pos < n {
                    if next >= cd_end {
                        return Err(bad("central directory truncated"));
                    }
                    buf.drain(..pos);
                    pos = 0;
                    let want = (n as u64).max(FIND_WINDOW);
                    let end = (next + want).min(cd_end);
                    let chunk = fetch(next..end).await?;
                    next = end;
                    buf.extend_from_slice(&chunk);
                }
            }};
        }
        ensure!(46);
        let rec_len = central_record_len(&buf[pos..])?;
        ensure!(rec_len);
        let mut c = Cursor::new(&buf[pos..pos + rec_len]);
        let entry = parse_central_record(&mut c)?;
        pos += rec_len;
        if entry.name == name {
            return Ok(Some(entry));
        }
    }
    Ok(None)
}

/// The byte range of `entry`'s (possibly compressed) body, found by reading
/// its 30-byte local header.
pub async fn entry_data_range<F, Fut>(
    entry: &CentralEntry,
    mut fetch: F,
) -> anyhow::Result<Range<u64>>
where
    F: FnMut(Range<u64>) -> Fut,
    Fut: Future<Output = anyhow::Result<Bytes>>,
{
    let at = entry.local_header_offset;
    let h = fetch(at..at + 30).await?;
    let mut c = Cursor::new(&h);
    if c.u32()? != LOCAL_HEADER_SIG {
        return Err(bad("bad local header signature"));
    }
    c.take(22)?;
    let name_len = c.u16()? as u64;
    let extra_len = c.u16()? as u64;
    let start = at + 30 + name_len + extra_len;
    Ok(start..start + entry.compressed_size)
}

/// Inflate a raw-deflate byte stream as it arrives.
pub fn inflate_stream(input: crate::ByteReadStream) -> crate::ByteReadStream {
    use futures_util::StreamExt;
    struct State {
        input: crate::ByteReadStream,
        inflater: flate2::Decompress,
        done: bool,
    }
    let state = State {
        input,
        inflater: flate2::Decompress::new(false),
        done: false,
    };
    Box::pin(futures_util::stream::unfold(state, |mut st| async move {
        loop {
            if st.done {
                return None;
            }
            let chunk = match st.input.next().await {
                Some(Ok(c)) => c,
                Some(Err(e)) => {
                    st.done = true;
                    return Some((Err(e), st));
                }
                None => return None,
            };
            let mut out = Vec::with_capacity(chunk.len() * 4 + 1024);
            let mut input: &[u8] = &chunk;
            loop {
                if out.capacity() - out.len() < 1024 {
                    out.reserve(64 * 1024);
                }
                let before = st.inflater.total_in();
                let status =
                    match st
                        .inflater
                        .decompress_vec(input, &mut out, flate2::FlushDecompress::None)
                    {
                        Ok(s) => s,
                        Err(e) => {
                            st.done = true;
                            return Some((Err(std::io::Error::other(e)), st));
                        }
                    };
                input = &input[(st.inflater.total_in() - before) as usize..];
                if status == flate2::Status::StreamEnd {
                    st.done = true;
                    break;
                }
                if input.is_empty() && out.len() < out.capacity() {
                    break;
                }
            }
            if !out.is_empty() {
                return Some((Ok(Bytes::from(out)), st));
            }
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{Rng, SeedableRng};
    use std::io::Read;

    async fn build(
        force_zip64: bool,
        spill: Option<&Path>,
    ) -> (Vec<u8>, Vec<(String, Vec<u8>, EntryDigest)>) {
        let mut rng = rand::rngs::StdRng::seed_from_u64(42);
        let mut zip = match spill {
            Some(dir) => ZipStreamWriter::with_spill(Vec::new(), dir).await.unwrap(),
            None => ZipStreamWriter::new(Vec::new()),
        };
        zip.force_zip64(force_zip64);
        let mut expected = Vec::new();
        let text: Vec<u8> = "name: hello\ntitle: Ünïcode\n".repeat(5_000).into_bytes();
        let random: Vec<u8> = (0..300_000).map(|_| rng.gen()).collect();
        let cases: Vec<(String, Vec<u8>, Compression)> = vec![
            (
                "manifest.yaml".into(),
                b"name: x\nversion: 1.0.0\n".to_vec(),
                Compression::Deflate(6),
            ),
            (
                "content/stories/a/.node.yaml".into(),
                text.clone(),
                Compression::Deflate(6),
            ),
            (
                "content/assets/photo.jpg".into(),
                random.clone(),
                Compression::Stored,
            ),
            (
                "content/assets/empty.bin".into(),
                Vec::new(),
                Compression::Stored,
            ),
            (
                "content/assets/empty.yaml".into(),
                Vec::new(),
                Compression::Deflate(6),
            ),
            (
                "content/stories/ü/ñ.yaml".into(),
                random.clone(),
                Compression::Deflate(9),
            ),
        ];
        for (name, data, comp) in cases {
            let mut e = zip.start_entry(&name, comp).await.unwrap();
            // Feed in uneven chunks.
            let mut i = 0;
            while i < data.len() {
                let n = rng.gen_range(1..20_000).min(data.len() - i);
                e.write(&data[i..i + n]).await.unwrap();
                i += n;
            }
            let digest = e.finish().await.unwrap();
            expected.push((name, data, digest));
        }
        (zip.finish().await.unwrap(), expected)
    }

    fn check_with_zip_crate(bytes: &[u8], expected: &[(String, Vec<u8>, EntryDigest)]) {
        use sha2::Digest;
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        assert_eq!(archive.len(), expected.len());
        for (name, data, digest) in expected {
            let mut f = archive.by_name(name).unwrap();
            let mut got = Vec::new();
            f.read_to_end(&mut got).unwrap();
            assert_eq!(&got, data, "{name}");
            assert_eq!(digest.size, data.len() as u64);
            assert_eq!(digest.sha256, hex::encode(sha2::Sha256::digest(data)));
        }
    }

    #[tokio::test]
    async fn the_zip_crate_reads_a_streamed_archive() {
        let (bytes, expected) = build(false, None).await;
        check_with_zip_crate(&bytes, &expected);
    }

    #[tokio::test]
    async fn the_zip64_layout_reads_back_too() {
        let (bytes, expected) = build(true, None).await;
        check_with_zip_crate(&bytes, &expected);
    }

    #[tokio::test]
    async fn a_spilled_central_directory_is_identical_and_cleaned_up() {
        let dir = tempfile::tempdir().unwrap();
        let (spilled, expected) = build(false, Some(dir.path())).await;
        check_with_zip_crate(&spilled, &expected);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn system_unzip_accepts_both_layouts() {
        // `unzip -t` is the strictest reader commonly installed; skip if absent.
        if std::process::Command::new("unzip")
            .arg("-v")
            .output()
            .is_err()
        {
            return;
        }
        for force in [false, true] {
            let (bytes, _) = build(force, None).await;
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("a.zip");
            std::fs::write(&path, &bytes).unwrap();
            let out = std::process::Command::new("unzip")
                .arg("-tq")
                .arg(&path)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "unzip -t (zip64={force}): {}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }

    #[tokio::test]
    async fn ranged_reads_find_and_inflate_one_entry() {
        use futures_util::StreamExt;
        for force in [false, true] {
            let (bytes, expected) = build(force, None).await;
            let bytes = Bytes::from(bytes);
            let fetched = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
            let fetch = |r: Range<u64>| {
                let b = bytes.clone();
                let fetched = fetched.clone();
                async move {
                    fetched.fetch_add(r.end - r.start, std::sync::atomic::Ordering::Relaxed);
                    Ok(b.slice(r.start as usize..r.end as usize))
                }
            };
            let cd = read_central_directory(bytes.len() as u64, fetch, 1 << 20)
                .await
                .unwrap();
            assert_eq!(cd.len(), expected.len());
            for (name, data, digest) in &expected {
                let entry = cd.iter().find(|e| &e.name == name).unwrap();
                assert_eq!(entry.size, data.len() as u64);
                assert_eq!(entry.crc32, digest.crc32);
                let range = entry_data_range(entry, fetch).await.unwrap();
                let body: crate::ByteReadStream = Box::pin(futures_util::stream::iter(
                    bytes
                        .slice(range.start as usize..range.end as usize)
                        .chunks(777)
                        .map(|c| Ok(Bytes::copy_from_slice(c)))
                        .collect::<Vec<_>>(),
                ));
                let body = if entry.is_deflated() {
                    inflate_stream(body)
                } else {
                    body
                };
                let got: Vec<u8> = body
                    .map(|c| c.unwrap().to_vec())
                    .collect::<Vec<_>>()
                    .await
                    .concat();
                assert_eq!(&got, data, "{name}");
            }
        }
    }

    #[tokio::test]
    async fn find_entry_walks_a_large_directory_in_small_windows() {
        let mut zip = ZipStreamWriter::new(Vec::new());
        for i in 0..5_000 {
            zip.write_entry(
                &format!("content/ws/node-{i:05}.yaml"),
                Compression::Stored,
                b"x",
            )
            .await
            .unwrap();
        }
        zip.write_entry("manifest.yaml", Compression::Deflate(6), b"name: big\n")
            .await
            .unwrap();
        let bytes = Bytes::from(zip.finish().await.unwrap());
        let largest = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let fetch = |r: Range<u64>| {
            let b = bytes.clone();
            let largest = largest.clone();
            async move {
                largest.fetch_max(r.end - r.start, std::sync::atomic::Ordering::Relaxed);
                Ok(b.slice(r.start as usize..r.end as usize))
            }
        };
        let e = find_entry(bytes.len() as u64, fetch, "manifest.yaml")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(e.size, 10);
        assert!(find_entry(bytes.len() as u64, fetch, "nope")
            .await
            .unwrap()
            .is_none());
        // The directory is ~400 KB; no single read was larger than a window
        // (or the tail read).
        assert!(
            largest.load(std::sync::atomic::Ordering::Relaxed) <= FIND_WINDOW.max(22 + 65_535 + 20)
        );
    }

    #[tokio::test]
    async fn an_oversized_central_directory_is_refused() {
        let (bytes, _) = build(false, None).await;
        let bytes = Bytes::from(bytes);
        let fetch = |r: Range<u64>| {
            let b = bytes.clone();
            async move { Ok(b.slice(r.start as usize..r.end as usize)) }
        };
        let err = read_central_directory(bytes.len() as u64, fetch, 10)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("central directory"));
    }

    #[test]
    fn media_is_stored_text_is_deflated() {
        assert_eq!(
            Compression::for_content_type(Some("image/jpeg")),
            Compression::Stored
        );
        assert_eq!(
            Compression::for_content_type(Some("video/mp4")),
            Compression::Stored
        );
        assert_eq!(
            Compression::for_content_type(Some("application/pdf")),
            Compression::Stored
        );
        assert_eq!(
            Compression::for_content_type(Some("image/svg+xml")),
            Compression::Deflate(6)
        );
        assert_eq!(Compression::for_content_type(None), Compression::Deflate(6));
    }
}
