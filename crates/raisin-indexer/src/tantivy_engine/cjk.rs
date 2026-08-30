// SPDX-License-Identifier: BSL-1.1

//! A script-aware word tokenizer.
//!
//! Tantivy's `SimpleTokenizer` splits only on non-alphanumeric characters. That
//! is fine for space-delimited scripts and catastrophic for the unspaced ones:
//! an ordinary Japanese or Chinese sentence contains no separator at all, so it
//! becomes ONE token. Combined with `RemoveLongFilter` — 40 bytes in tantivy's
//! `"default"` analyzer — any CJK sentence past ~13 characters was not merely
//! ranked badly, it was **deleted at index time**.
//!
//! [`ScriptAwareTokenizer`] keeps `SimpleTokenizer`'s behaviour for everything
//! else and segments Han / Hiragana / Katakana runs into **overlapping
//! bigrams**, which is what Lucene's `CJKAnalyzer` has always done. Bigrams are
//! chosen over a dictionary segmenter (Lindera, Jieba) deliberately:
//!
//! * they need no dictionary — no extra crate, no multi-megabyte data file, and
//!   no model-versioning problem where a dictionary upgrade silently changes
//!   the meaning of every segment already on disk;
//! * recall is what this full-text path is for, and bigrams over-generate
//!   rather than under-generate (a dictionary segmenter that guesses a word
//!   boundary wrong makes a document permanently unfindable);
//! * index and query go through the SAME analyzer, so the over-generation is
//!   symmetric and matching stays consistent.
//!
//! The tradeoff is precision: `東京都` yields `東京` and `京都`, so a search for
//! `京都` (Kyoto) can surface a document about `東京都` (Tokyo Metropolis). A
//! dictionary segmenter is the fix for that, and is the upgrade path if
//! precision ever matters more than recall here.
//!
//! Hangul is intentionally NOT bigrammed: Korean is space-delimited, so the
//! word path already segments it correctly. Thai / Lao / Khmer are unspaced but
//! have no bigram tradition and stay on the word path — a known remaining gap,
//! now bounded by the byte limit instead of deleted by it.

use tantivy::tokenizer::{Token, TokenStream, Tokenizer};

/// True for characters in a script written without spaces AND for which
/// overlapping bigrams are the accepted dictionary-free segmentation.
fn is_bigrammable_cjk(c: char) -> bool {
    matches!(c as u32,
        0x3005                    // 々 ideographic iteration mark
        | 0x3007                  // 〇 ideographic number zero
        | 0x3040..=0x30FF         // Hiragana + Katakana
        | 0x31F0..=0x31FF         // Katakana phonetic extensions
        | 0x3400..=0x4DBF         // CJK Unified Ideographs Extension A
        | 0x4E00..=0x9FFF         // CJK Unified Ideographs
        | 0xF900..=0xFAFF         // CJK Compatibility Ideographs
        | 0xFF66..=0xFF9D         // Halfwidth Katakana
        | 0x20000..=0x2FA1F       // CJK Ext. B..F and Compatibility Supplement
    )
}

/// Word-and-bigram tokenizer. See the module docs.
#[derive(Clone, Default)]
pub(crate) struct ScriptAwareTokenizer;

/// A token stream over a pre-computed token vector.
///
/// Building the vector up front keeps the segmentation in one readable pass;
/// what an analyzer is handed here is a single node field, not a stream, so
/// there is nothing to gain from laziness.
pub(crate) struct PrecomputedTokenStream {
    tokens: std::vec::IntoIter<Token>,
    current: Token,
}

impl TokenStream for PrecomputedTokenStream {
    fn advance(&mut self) -> bool {
        match self.tokens.next() {
            Some(token) => {
                self.current = token;
                true
            }
            None => false,
        }
    }

    fn token(&self) -> &Token {
        &self.current
    }

    fn token_mut(&mut self) -> &mut Token {
        &mut self.current
    }
}

impl Tokenizer for ScriptAwareTokenizer {
    type TokenStream<'a> = PrecomputedTokenStream;

    fn token_stream<'a>(&'a mut self, text: &'a str) -> Self::TokenStream<'a> {
        PrecomputedTokenStream {
            tokens: segment(text).into_iter(),
            current: Token::default(),
        }
    }
}

/// One pass over the text, emitting Latin-style word tokens and CJK bigrams.
fn segment(text: &str) -> Vec<Token> {
    let mut tokens: Vec<Token> = Vec::new();
    let mut position: usize = 0;

    // (byte offset, char) of the run being accumulated, and its script class.
    let mut run: Vec<(usize, char)> = Vec::new();
    let mut run_is_cjk = false;

    for (offset, c) in text.char_indices() {
        let cjk = is_bigrammable_cjk(c);
        let word_char = cjk || c.is_alphanumeric();

        if !word_char {
            flush_run(text, &mut run, run_is_cjk, &mut tokens, &mut position);
            continue;
        }
        if !run.is_empty() && cjk != run_is_cjk {
            flush_run(text, &mut run, run_is_cjk, &mut tokens, &mut position);
        }
        run_is_cjk = cjk;
        run.push((offset, c));
    }
    flush_run(text, &mut run, run_is_cjk, &mut tokens, &mut position);

    tokens
}

fn flush_run(
    text: &str,
    run: &mut Vec<(usize, char)>,
    is_cjk: bool,
    tokens: &mut Vec<Token>,
    position: &mut usize,
) {
    if run.is_empty() {
        return;
    }
    if is_cjk && run.len() > 1 {
        for window in run.windows(2) {
            push_token(text, window[0].0, window[1], tokens, position);
        }
    } else if is_cjk {
        push_token(text, run[0].0, run[0], tokens, position);
    } else {
        push_token(text, run[0].0, run[run.len() - 1], tokens, position);
    }
    run.clear();
}

fn push_token(
    text: &str,
    from: usize,
    last: (usize, char),
    tokens: &mut Vec<Token>,
    position: &mut usize,
) {
    let (last_offset, last_char) = last;
    let to = last_offset + last_char.len_utf8();
    tokens.push(Token {
        offset_from: from,
        offset_to: to,
        position: *position,
        text: text[from..to].to_string(),
        position_length: 1,
    });
    *position += 1;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(text: &str) -> Vec<String> {
        let mut tokenizer = ScriptAwareTokenizer;
        let mut stream = tokenizer.token_stream(text);
        let mut out = Vec::new();
        while stream.advance() {
            out.push(stream.token().text.clone());
        }
        out
    }

    #[test]
    fn latin_text_is_split_on_separators_like_simple_tokenizer() {
        assert_eq!(
            tokens("Hello, world! RaisinDB-2"),
            vec!["Hello", "world", "RaisinDB", "2"]
        );
    }

    #[test]
    fn an_unspaced_japanese_run_becomes_overlapping_bigrams() {
        // Previously ONE 15-byte-per-5-chars token; a real sentence blew past
        // RemoveLongFilter's 40 bytes and was deleted outright.
        let got = tokens("情報を保存");
        assert_eq!(got, vec!["情報", "報を", "を保", "保存"]);
        assert!(got.iter().all(|t| t.len() <= 12));
    }

    #[test]
    fn a_single_cjk_character_yields_a_unigram() {
        assert_eq!(tokens("犬"), vec!["犬"]);
    }

    #[test]
    fn mixed_script_runs_are_segmented_independently() {
        assert_eq!(tokens("RaisinDBは情報"), vec!["RaisinDB", "は情", "情報"]);
    }

    #[test]
    fn hangul_stays_on_the_word_path() {
        assert_eq!(tokens("한국어 데이터"), vec!["한국어", "데이터"]);
    }

    #[test]
    fn offsets_point_back_at_the_original_text() {
        let text = "ab 情報";
        let mut tokenizer = ScriptAwareTokenizer;
        let mut stream = tokenizer.token_stream(text);
        let mut seen = 0;
        while stream.advance() {
            let token = stream.token();
            assert_eq!(&text[token.offset_from..token.offset_to], token.text);
            seen += 1;
        }
        assert_eq!(seen, 2);
    }

    #[test]
    fn positions_increase_by_one_per_token() {
        let text = "情報を保存 する";
        let mut tokenizer = ScriptAwareTokenizer;
        let mut stream = tokenizer.token_stream(text);
        let mut expected = 0;
        while stream.advance() {
            assert_eq!(stream.token().position, expected);
            expected += 1;
        }
        assert!(expected > 1);
    }
}
