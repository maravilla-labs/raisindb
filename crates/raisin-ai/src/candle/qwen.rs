//! Qwen2 text generation on Candle — in-process chat completion, no network.
//!
//! The rest of `candle/` is vision (CLIP, BLIP, Moondream), so this is the
//! first model here that answers a text prompt. It exists because `local:` is
//! the ONLY provider route that needs no tenant AI configuration: a provider
//! entry is tenant state that `deploy` cannot create, and a missing one fails
//! as an empty answer rather than an error. A local text model is therefore
//! the only way an installation can generate anything on first boot.
//!
//! ZERO NEW DEPENDENCIES. `candle-transformers` already ships
//! `quantized_qwen2`, and the HuggingFace registry already downloads GGUF plus
//! `tokenizer.json`, so this is glue: read the GGUF, build the ChatML prompt,
//! run the decode loop, stop at the right token.
//!
//! ── Two things here are correctness, not style ───────────────────────────
//!
//! 1. THE KV CACHE MAKES `index_pos` LOAD-BEARING. `forward` is called once
//!    with the whole prompt at position 0, then once per generated token with
//!    that single token and its true absolute position. Feeding the full
//!    sequence every step would be quadratic AND wrong, because the cache
//!    already holds the earlier keys.
//!
//! 2. STOPPING IS BY TOKEN, NOT BY TEXT. Qwen ends a turn with `<|im_end|>`,
//!    which decodes to a STRING that a caller would otherwise have to strip —
//!    and a model that emits the characters `<|im_end|>` without the special
//!    token is not ending its turn. Comparing token ids keeps those apart.

use std::path::Path;
use std::time::Instant;

use candle_core::quantized::gguf_file;
use candle_core::{Device, Tensor};
use candle_transformers::generation::LogitsProcessor;
use candle_transformers::models::quantized_qwen2::ModelWeights;
use tokenizers::Tokenizer;

use super::{CandleError, CandleResult};

/// The default local coder model: Qwen2.5-Coder 1.5B Instruct, Q4_K_M GGUF.
///
/// 1.5B and quantized on purpose. This runs on the machine serving Studio,
/// beside everything else it is doing, so a model that needs several GB of
/// residency is not a default anybody can ship.
pub const QWEN_CODER_MODEL: &str = "Qwen/Qwen2.5-Coder-1.5B-Instruct-GGUF";
/// The GGUF file inside that repo.
pub const QWEN_CODER_GGUF: &str = "qwen2.5-coder-1.5b-instruct-q4_k_m.gguf";

/// How many tokens of history the repeat penalty looks back over.
const REPEAT_LAST_N: usize = 64;
const REPEAT_PENALTY: f32 = 1.1;
/// A ceiling so a degenerate model cannot spin forever.
const MAX_NEW_TOKENS: usize = 4096;
/// Wall-clock budget for one generation.
///
/// A TIME budget rather than only a token budget, because the thing that hurts
/// is seconds, not tokens: this model runs on whatever CPU or GPU the server
/// has, so the same 2000-token answer is fifteen seconds on one machine and
/// fifteen minutes on another. A token cap cannot tell those apart.
///
/// Observed before this existed: a request the client had already given up on
/// kept six cores busy, because nothing downstream of the abandoned connection
/// knew to stop.
const TIME_BUDGET: std::time::Duration = std::time::Duration::from_secs(90);

/// One chat turn, in the shape the provider hands over.
pub struct ChatTurn<'a> {
    pub role: &'a str,
    pub content: &'a str,
}

/// A loaded Qwen2 model, ready to answer.
///
/// Holds its own KV cache inside `ModelWeights`, so it is `&mut self` to
/// generate and lives behind a `Mutex` in the provider — one conversation at a
/// time per process. That is the honest shape for a single in-process model,
/// and it is why the provider caches exactly one.
pub struct QwenGenerator {
    model: ModelWeights,
    tokenizer: Tokenizer,
    device: Device,
    /// Both tokens that end a turn: `<|im_end|>` for chat, `<|endoftext|>` for
    /// the base model. Either means stop.
    eos_tokens: Vec<u32>,
}

impl QwenGenerator {
    /// Load from a GGUF file and its tokenizer.
    pub fn new(gguf_path: &Path, tokenizer_path: &Path, device: Device) -> CandleResult<Self> {
        if !gguf_path.exists() {
            return Err(CandleError::ModelNotDownloaded(format!(
                "Qwen GGUF not found at {:?}",
                gguf_path
            )));
        }
        if !tokenizer_path.exists() {
            return Err(CandleError::ModelNotDownloaded(format!(
                "Tokenizer not found at {:?}. The GGUF alone is not enough.",
                tokenizer_path
            )));
        }

        let tokenizer = Tokenizer::from_file(tokenizer_path)
            .map_err(|e| CandleError::Tokenization(format!("Failed to load tokenizer: {}", e)))?;

        let mut file = std::fs::File::open(gguf_path)
            .map_err(|e| CandleError::ModelLoad(format!("Failed to open GGUF: {}", e)))?;
        // The GGUF header carries the architecture config (head counts, rope
        // base, context length), so there is no separate Config to keep in step
        // with the file — read it from the same bytes the weights come from.
        let content = gguf_file::Content::read(&mut file)
            .map_err(|e| CandleError::ModelLoad(format!("Failed to read GGUF header: {}", e)))?;
        let model = ModelWeights::from_gguf(content, &mut file, &device)
            .map_err(|e| CandleError::ModelLoad(format!("Failed to build Qwen weights: {}", e)))?;

        let eos_tokens: Vec<u32> = ["<|im_end|>", "<|endoftext|>"]
            .iter()
            .filter_map(|t| tokenizer.token_to_id(t))
            .collect();
        if eos_tokens.is_empty() {
            return Err(CandleError::Tokenization(
                "Tokenizer has neither <|im_end|> nor <|endoftext|>; this is not a Qwen chat model."
                    .to_string(),
            ));
        }

        tracing::info!(
            gguf = ?gguf_path,
            device = ?device,
            eos = ?eos_tokens,
            "Qwen2 model loaded"
        );

        Ok(Self {
            model,
            tokenizer,
            device,
            eos_tokens,
        })
    }

    /// Render turns as ChatML, the format Qwen2.5-Instruct was trained on.
    ///
    /// The trailing `<|im_start|>assistant\n` is the generation prompt: without
    /// it the model continues the USER's turn instead of answering it, which
    /// looks like a model that ignores instructions rather than a formatting
    /// bug.
    pub fn build_prompt(turns: &[ChatTurn<'_>]) -> String {
        let mut out = String::new();
        for t in turns {
            out.push_str("<|im_start|>");
            out.push_str(t.role);
            out.push('\n');
            out.push_str(t.content);
            out.push_str("<|im_end|>\n");
        }
        out.push_str("<|im_start|>assistant\n");
        out
    }

    /// Generate a completion for a rendered prompt.
    ///
    /// `temperature` of 0 (or None) is greedy, which is what structured output
    /// wants: the caller is usually asking for JSON against a schema, and a
    /// sampled token that breaks it costs a whole retry.
    pub fn generate(
        &mut self,
        prompt: &str,
        max_tokens: usize,
        temperature: Option<f64>,
        seed: u64,
    ) -> CandleResult<String> {
        let encoding = self
            .tokenizer
            .encode(prompt, true)
            .map_err(|e| CandleError::Tokenization(format!("Failed to encode prompt: {}", e)))?;
        let prompt_tokens = encoding.get_ids().to_vec();
        if prompt_tokens.is_empty() {
            return Err(CandleError::Tokenization("Empty prompt".to_string()));
        }

        let temp = match temperature {
            Some(t) if t > 0.0 => Some(t),
            _ => None,
        };
        let mut logits_processor = LogitsProcessor::new(seed, temp, None);

        // ── Prefill: the whole prompt in one forward, at position 0. ──────
        let input = Tensor::new(prompt_tokens.as_slice(), &self.device)
            .and_then(|t| t.unsqueeze(0))
            .map_err(|e| CandleError::Inference(format!("Failed to build input tensor: {}", e)))?;
        let mut logits = self
            .model
            .forward(&input, 0)
            .map_err(|e| CandleError::Inference(format!("Prefill failed: {}", e)))?;
        logits = squeeze_last(logits)?;

        let budget = max_tokens.clamp(1, MAX_NEW_TOKENS);
        let deadline = Instant::now() + TIME_BUDGET;
        let started = Instant::now();
        let mut generated: Vec<u32> = Vec::with_capacity(budget);
        // Absolute position of the NEXT token — the KV cache already holds the
        // prompt, so decoding continues from the end of it.
        let mut index_pos = prompt_tokens.len();

        for _ in 0..budget {
            if Instant::now() >= deadline {
                return Err(CandleError::Inference(format!(
                    "Generation exceeded {}s after {} tokens ({:.1} tok/s). This machine is too \
                     slow for this model, or the answer was going to be very long.",
                    TIME_BUDGET.as_secs(),
                    generated.len(),
                    generated.len() as f64 / started.elapsed().as_secs_f64().max(0.001),
                )));
            }
            let adjusted = if REPEAT_PENALTY == 1.0 || generated.is_empty() {
                logits.clone()
            } else {
                let start = generated.len().saturating_sub(REPEAT_LAST_N);
                candle_transformers::utils::apply_repeat_penalty(
                    &logits,
                    REPEAT_PENALTY,
                    &generated[start..],
                )
                .map_err(|e| CandleError::Inference(format!("Repeat penalty failed: {}", e)))?
            };

            let next = logits_processor
                .sample(&adjusted)
                .map_err(|e| CandleError::Inference(format!("Sampling failed: {}", e)))?;

            // BEFORE pushing: an end-of-turn token is a control signal, not
            // output, and decoding it would put a literal "<|im_end|>" into the
            // answer.
            if self.eos_tokens.contains(&next) {
                break;
            }
            generated.push(next);

            let input = Tensor::new(&[next], &self.device)
                .and_then(|t| t.unsqueeze(0))
                .map_err(|e| {
                    CandleError::Inference(format!("Failed to build step tensor: {}", e))
                })?;
            logits = self
                .model
                .forward(&input, index_pos)
                .map_err(|e| CandleError::Inference(format!("Decode failed: {}", e)))?;
            logits = squeeze_last(logits)?;
            index_pos += 1;
        }

        // Logged so the real speed of a given box is discoverable rather than
        // guessed at — it is the number that decides whether a local model is
        // usable here at all.
        tracing::info!(
            tokens = generated.len(),
            secs = started.elapsed().as_secs_f64(),
            tok_per_s = generated.len() as f64 / started.elapsed().as_secs_f64().max(0.001),
            "Qwen generation complete"
        );

        self.tokenizer
            .decode(&generated, true)
            .map_err(|e| CandleError::Tokenization(format!("Failed to decode output: {}", e)))
    }
}

/// Reduce the model's output to the logits for the LAST position, as f32.
///
/// `forward` returns `[batch, seq, vocab]` for a prefill and `[batch, 1, vocab]`
/// for a decode step; the sampler wants a flat `[vocab]`. Taking the last row
/// rather than assuming `seq == 1` is what lets the same helper serve both.
fn squeeze_last(logits: Tensor) -> CandleResult<Tensor> {
    let logits = if logits.rank() == 3 {
        let seq = logits
            .dim(1)
            .map_err(|e| CandleError::Inference(format!("Bad logits shape: {}", e)))?;
        logits
            .i((0, seq - 1))
            .map_err(|e| CandleError::Inference(format!("Failed to slice logits: {}", e)))?
    } else {
        logits
            .squeeze(0)
            .map_err(|e| CandleError::Inference(format!("Failed to squeeze logits: {}", e)))?
    };
    logits
        .to_dtype(candle_core::DType::F32)
        .map_err(|e| CandleError::Inference(format!("Failed to cast logits: {}", e)))
}

// `i()` (indexing) comes from this trait.
use candle_core::IndexOp;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chatml_has_a_generation_prompt() {
        let p = QwenGenerator::build_prompt(&[
            ChatTurn {
                role: "system",
                content: "You are terse.",
            },
            ChatTurn {
                role: "user",
                content: "Hi",
            },
        ]);
        assert_eq!(
            p,
            "<|im_start|>system\nYou are terse.<|im_end|>\n\
             <|im_start|>user\nHi<|im_end|>\n\
             <|im_start|>assistant\n"
        );
        // Without this the model continues the user's turn instead of replying.
        assert!(p.ends_with("<|im_start|>assistant\n"));
    }

    #[test]
    fn empty_turns_still_ask_for_an_answer() {
        assert_eq!(QwenGenerator::build_prompt(&[]), "<|im_start|>assistant\n");
    }
}
