//! SHA-256 and byte count, computed in the same pass that moves the bytes.
//!
//! A generated archive's checksum and exact size must come from the stream
//! that wrote it, never from re-reading what was stored: re-reading a
//! multi-gigabyte object doubles the I/O and can disagree with what was sent.

use sha2::{Digest as _, Sha256};
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::AsyncWrite;

/// The digest of everything that went through a [`HashingWriter`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamDigest {
    /// Lower-case hex SHA-256.
    pub sha256: String,
    pub size: u64,
}

impl StreamDigest {
    /// The digest as `Repr-Digest` / `Content-Digest` want it (RFC 9530):
    /// standard base64 of the raw 32 bytes.
    pub fn sha256_base64(&self) -> Option<String> {
        sha256_hex_to_base64(&self.sha256)
    }
}

/// Convert a lower-case hex SHA-256 to standard base64, `None` if not 32 bytes of hex.
pub fn sha256_hex_to_base64(hex_digest: &str) -> Option<String> {
    let raw = hex::decode(hex_digest).ok()?;
    if raw.len() != 32 {
        return None;
    }
    Some(base64_encode(&raw))
}

fn base64_encode(raw: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(raw.len().div_ceil(3) * 4);
    for chunk in raw.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// Incremental SHA-256 plus a byte counter, for callers that see chunks.
#[derive(Clone, Default)]
pub struct StreamHasher {
    hasher: Sha256,
    size: u64,
}

impl StreamHasher {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update(&mut self, bytes: &[u8]) {
        self.hasher.update(bytes);
        self.size += bytes.len() as u64;
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    pub fn finish(self) -> StreamDigest {
        StreamDigest {
            sha256: hex::encode(self.hasher.finalize()),
            size: self.size,
        }
    }
}

/// An [`AsyncWrite`] tee: everything the inner writer *accepts* is hashed and
/// counted. Bytes the inner writer has not taken yet are not counted, so a
/// short write never skews the digest.
pub struct HashingWriter<W> {
    inner: W,
    hasher: StreamHasher,
}

impl<W> HashingWriter<W> {
    pub fn new(inner: W) -> Self {
        Self {
            inner,
            hasher: StreamHasher::new(),
        }
    }

    pub fn bytes_written(&self) -> u64 {
        self.hasher.size()
    }

    pub fn get_ref(&self) -> &W {
        &self.inner
    }

    pub fn get_mut(&mut self) -> &mut W {
        &mut self.inner
    }

    pub fn finish(self) -> (W, StreamDigest) {
        (self.inner, self.hasher.finish())
    }
}

impl<W: AsyncWrite + Unpin> AsyncWrite for HashingWriter<W> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let this = &mut *self;
        match Pin::new(&mut this.inner).poll_write(cx, buf) {
            Poll::Ready(Ok(n)) => {
                this.hasher.update(&buf[..n]);
                Poll::Ready(Ok(n))
            }
            other => other,
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{Rng, SeedableRng};
    use tokio::io::AsyncWriteExt;

    /// A writer that accepts at most `max` bytes per call, to exercise short writes.
    struct Trickle {
        out: Vec<u8>,
        max: usize,
    }

    impl AsyncWrite for Trickle {
        fn poll_write(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            let n = buf.len().min(self.max);
            self.out.extend_from_slice(&buf[..n]);
            Poll::Ready(Ok(n))
        }
        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn the_digest_matches_sha2_over_random_chunk_splits() {
        let mut rng = rand::rngs::StdRng::seed_from_u64(7);
        let data: Vec<u8> = (0..200_000).map(|_| rng.gen()).collect();
        let expected = hex::encode(Sha256::digest(&data));

        for round in 0..20 {
            let mut w = HashingWriter::new(Trickle {
                out: Vec::new(),
                max: 1 + round * 37,
            });
            let mut i = 0;
            while i < data.len() {
                let n = rng.gen_range(1..=4096).min(data.len() - i);
                w.write_all(&data[i..i + n]).await.unwrap();
                i += n;
            }
            let (inner, digest) = w.finish();
            assert_eq!(digest.sha256, expected);
            assert_eq!(digest.size, data.len() as u64);
            assert_eq!(inner.out, data);
        }
    }

    #[test]
    fn base64_digest_is_rfc9530_shaped() {
        // sha256("") — the standard test vector.
        let d = StreamHasher::new().finish();
        assert_eq!(
            d.sha256,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            d.sha256_base64().unwrap(),
            "47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU="
        );
        assert!(sha256_hex_to_base64("abcd").is_none());
    }
}
