// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Single-range `Range: bytes=` resolution for served assets.
//!
//! # Why an asset server needs this at all
//!
//! Without it a `<video>` or `<audio>` element can take the whole body or
//! nothing. The element PROBES first — a HEAD, or a tiny opening range — and
//! decides from `Accept-Ranges` whether the scrub bar can work; a server that
//! answers 200 with the entire file to `bytes=1000000-` has told the browser
//! that seeking is impossible, and the control is dead for every mp3 and mp4 in
//! the library. That is why `Accept-Ranges: bytes` goes on the FULL response
//! too, not only on partials.
//!
//! # Scope
//!
//! Exactly one `bytes=` range, which is the only shape browsers send for
//! seeking. Multiple ranges in one header would require a
//! `multipart/byteranges` body, and a client that asks for several ranges must
//! accept a 200 with the whole entity (RFC 9110 §14.2 makes a range request an
//! optimisation the server may decline). So a multi-range header resolves to
//! [`RangeResolution::None`] and we serve the FULL body — never the first range
//! alone, which would be a 206 whose `Content-Range` silently contradicts what
//! the client asked for and truncates the entity it reassembles.
//!
//! Same shape and the same scope decisions as the proven implementation in the
//! delivery service (`delivery/src/utils/http_range.rs`), rewritten for this
//! crate's types rather than copied.

/// Outcome of resolving a `Range` request header against a known total size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RangeResolution {
    /// No usable `Range` header — serve the whole body as `200`.
    None,
    /// A satisfiable byte range — serve `[start, end]` INCLUSIVE as `206`.
    Satisfiable { start: u64, end: u64 },
    /// Syntactically valid but unsatisfiable — respond `416`.
    ///
    /// Distinct from `None` on purpose: a malformed header is a client that
    /// does not know how to ask, and serving it everything is correct; a
    /// well-formed request for byte 5000 of a 1000-byte file is a client that
    /// believes something false, and answering 200 leaves it believing it.
    Unsatisfiable,
}

/// Resolve a single-range `bytes=` header against `total`, the full entity size.
///
/// The three forms a browser sends:
/// - `bytes=START-END` — an explicit closed range
/// - `bytes=START-`    — from START to the end
/// - `bytes=-SUFFIX`   — the last SUFFIX bytes
///
/// `end` is clamped to `total - 1`, because a client asking past the end of a
/// file it has not measured yet is ordinary, not an error. Anything malformed,
/// multi-range, or in a unit other than `bytes` yields [`RangeResolution::None`]
/// so the caller serves the full body — always a valid answer to a range
/// request. `total == 0` yields `None`: there is nothing to range over, and
/// every arithmetic path below would underflow on `total - 1`.
pub(crate) fn resolve(header: Option<&str>, total: u64) -> RangeResolution {
    let Some(raw) = header else {
        return RangeResolution::None;
    };
    if total == 0 {
        return RangeResolution::None;
    }
    let Some(spec) = raw.trim().strip_prefix("bytes=") else {
        return RangeResolution::None;
    };
    // A comma means multipart/byteranges, which we do not emit — see the
    // module docs for why that is a full body and not the first range.
    if spec.contains(',') {
        return RangeResolution::None;
    }
    let Some((start_s, end_s)) = spec.split_once('-') else {
        return RangeResolution::None;
    };
    let start_s = start_s.trim();
    let end_s = end_s.trim();

    let (start, end) = if start_s.is_empty() {
        // Suffix range: the last N bytes.
        let Ok(suffix) = end_s.parse::<u64>() else {
            return RangeResolution::None;
        };
        // `bytes=-0` asks for the last zero bytes, which no response can
        // satisfy — 416 rather than an empty 206.
        if suffix == 0 {
            return RangeResolution::Unsatisfiable;
        }
        let suffix = suffix.min(total);
        (total - suffix, total - 1)
    } else {
        let Ok(start) = start_s.parse::<u64>() else {
            return RangeResolution::None;
        };
        if start >= total {
            return RangeResolution::Unsatisfiable;
        }
        let end = if end_s.is_empty() {
            total - 1
        } else {
            match end_s.parse::<u64>() {
                Ok(e) => e.min(total - 1),
                Err(_) => return RangeResolution::None,
            }
        };
        if end < start {
            return RangeResolution::Unsatisfiable;
        }
        (start, end)
    };

    RangeResolution::Satisfiable { start, end }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_header_serves_the_whole_body() {
        assert_eq!(resolve(None, 1000), RangeResolution::None);
    }

    #[test]
    fn closed_range() {
        assert_eq!(
            resolve(Some("bytes=0-499"), 1000),
            RangeResolution::Satisfiable { start: 0, end: 499 }
        );
    }

    #[test]
    fn open_ended_range_runs_to_eof() {
        assert_eq!(
            resolve(Some("bytes=500-"), 1000),
            RangeResolution::Satisfiable {
                start: 500,
                end: 999
            }
        );
    }

    /// A player seeking in a file whose length it has not learned yet asks past
    /// the end routinely. Clamp, do not refuse.
    #[test]
    fn an_end_past_eof_clamps() {
        assert_eq!(
            resolve(Some("bytes=900-100000"), 1000),
            RangeResolution::Satisfiable {
                start: 900,
                end: 999
            }
        );
    }

    #[test]
    fn suffix_range_takes_the_tail() {
        assert_eq!(
            resolve(Some("bytes=-200"), 1000),
            RangeResolution::Satisfiable {
                start: 800,
                end: 999
            }
        );
    }

    #[test]
    fn a_suffix_larger_than_the_file_is_the_whole_file() {
        assert_eq!(
            resolve(Some("bytes=-5000"), 1000),
            RangeResolution::Satisfiable { start: 0, end: 999 }
        );
    }

    #[test]
    fn a_start_past_eof_is_unsatisfiable() {
        assert_eq!(
            resolve(Some("bytes=2000-"), 1000),
            RangeResolution::Unsatisfiable
        );
    }

    #[test]
    fn an_inverted_range_is_unsatisfiable() {
        assert_eq!(
            resolve(Some("bytes=500-100"), 1000),
            RangeResolution::Unsatisfiable
        );
    }

    #[test]
    fn a_zero_length_suffix_is_unsatisfiable() {
        assert_eq!(
            resolve(Some("bytes=-0"), 1000),
            RangeResolution::Unsatisfiable
        );
    }

    /// Not the first range: a 206 for one of several ranges truncates the
    /// entity the client reassembles, with a `Content-Range` that says so and
    /// is believed.
    #[test]
    fn a_multi_range_header_serves_the_whole_body() {
        assert_eq!(
            resolve(Some("bytes=0-99,200-299"), 1000),
            RangeResolution::None
        );
    }

    #[test]
    fn a_non_bytes_unit_serves_the_whole_body() {
        assert_eq!(resolve(Some("items=0-99"), 1000), RangeResolution::None);
    }

    /// `total - 1` underflows for an empty entity, so this must short-circuit
    /// before any arithmetic.
    #[test]
    fn an_empty_entity_has_nothing_to_range_over() {
        assert_eq!(resolve(Some("bytes=0-10"), 0), RangeResolution::None);
        assert_eq!(resolve(Some("bytes=-10"), 0), RangeResolution::None);
    }

    #[test]
    fn garbage_serves_the_whole_body_rather_than_failing() {
        for h in ["bytes=", "bytes=abc-def", "bytes=x", "", "bytes=-"] {
            assert_eq!(
                resolve(Some(h), 1000),
                RangeResolution::None,
                "header {h:?}"
            );
        }
    }
}
