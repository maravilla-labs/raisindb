//! Token generation for standard and quantized Moondream models.

use candle_core::{Device, IndexOp, Tensor};
use candle_transformers::models::moondream;
use candle_transformers::models::quantized_moondream;

use super::super::{CandleError, CandleResult};

/// Generate tokens using standard (non-quantized) model.
pub(crate) fn generate_tokens_standard(
    model: &mut moondream::Model,
    bos_token: &Tensor,
    prompt_token_ids: &[u32],
    image_embeds: &Tensor,
    max_length: usize,
    device: &Device,
    eos_token_id: u32,
) -> CandleResult<Vec<u32>> {
    let mut generated_tokens = Vec::new();
    let mut all_tokens = prompt_token_ids.to_vec();

    for index in 0..max_length {
        let logits = if index == 0 {
            // First iteration: use forward_with_img
            let input_ids = Tensor::new(prompt_token_ids, device)
                .map_err(|e| CandleError::Inference(format!("Input tensor failed: {}", e)))?
                .unsqueeze(0)
                .map_err(|e| CandleError::Inference(format!("Unsqueeze failed: {}", e)))?;

            model
                .text_model
                .forward_with_img(bos_token, &input_ids, image_embeds)
                .map_err(|e| CandleError::Inference(format!("Text forward failed: {}", e)))?
        } else {
            // Subsequent iterations: use regular forward
            let last_token = all_tokens.last().copied().unwrap_or(eos_token_id);
            let input_ids = Tensor::new(&[last_token], device)
                .map_err(|e| CandleError::Inference(format!("Input tensor failed: {}", e)))?
                .unsqueeze(0)
                .map_err(|e| CandleError::Inference(format!("Unsqueeze failed: {}", e)))?;

            model
                .text_model
                .forward(&input_ids)
                .map_err(|e| CandleError::Inference(format!("Text forward failed: {}", e)))?
        };

        let next_token = extract_next_token(&logits, eos_token_id)?;

        // Stop if we hit EOS
        if next_token == eos_token_id {
            break;
        }

        generated_tokens.push(next_token);
        all_tokens.push(next_token);
    }

    Ok(generated_tokens)
}

/// Generate tokens using quantized model.
pub(crate) fn generate_tokens_quantized(
    model: &mut quantized_moondream::Model,
    bos_token: &Tensor,
    prompt_token_ids: &[u32],
    image_embeds: &Tensor,
    max_length: usize,
    device: &Device,
    eos_token_id: u32,
) -> CandleResult<Vec<u32>> {
    let mut generated_tokens = Vec::new();
    let mut all_tokens = prompt_token_ids.to_vec();

    for index in 0..max_length {
        let logits = if index == 0 {
            // First iteration: use forward_with_img
            let input_ids = Tensor::new(prompt_token_ids, device)
                .map_err(|e| CandleError::Inference(format!("Input tensor failed: {}", e)))?
                .unsqueeze(0)
                .map_err(|e| CandleError::Inference(format!("Unsqueeze failed: {}", e)))?;

            model
                .text_model
                .forward_with_img(bos_token, &input_ids, image_embeds)
                .map_err(|e| CandleError::Inference(format!("Text forward failed: {}", e)))?
        } else {
            // Subsequent iterations: use regular forward
            let last_token = all_tokens.last().copied().unwrap_or(eos_token_id);
            let input_ids = Tensor::new(&[last_token], device)
                .map_err(|e| CandleError::Inference(format!("Input tensor failed: {}", e)))?
                .unsqueeze(0)
                .map_err(|e| CandleError::Inference(format!("Unsqueeze failed: {}", e)))?;

            model
                .text_model
                .forward(&input_ids)
                .map_err(|e| CandleError::Inference(format!("Text forward failed: {}", e)))?
        };

        let next_token = extract_next_token(&logits, eos_token_id)?;

        // Stop if we hit EOS
        if next_token == eos_token_id {
            break;
        }

        generated_tokens.push(next_token);
        all_tokens.push(next_token);
    }

    Ok(generated_tokens)
}

/// Extract the next token from logits using greedy decoding.
///
/// Handles variable tensor shapes: [batch, seq, vocab] or [seq, vocab].
fn extract_next_token(logits: &Tensor, _eos_token_id: u32) -> CandleResult<u32> {
    let logits_shape = logits.dims();
    let last_logits = match logits_shape.len() {
        3 => {
            // Shape: [batch, seq_len, vocab] - get last position
            let seq_len = logits_shape[1];
            logits
                .i((0, seq_len - 1, ..))
                .map_err(|e| CandleError::Inference(format!("Index 3D failed: {}", e)))?
        }
        2 => {
            // Shape: [seq_len, vocab] - get last position
            let seq_len = logits_shape[0];
            logits
                .i((seq_len - 1, ..))
                .map_err(|e| CandleError::Inference(format!("Index 2D failed: {}", e)))?
        }
        _ => {
            return Err(CandleError::Inference(format!(
                "Unexpected logits shape: {:?}",
                logits_shape
            )));
        }
    };

    // Greedy decoding: take argmax
    last_logits
        .argmax(candle_core::D::Minus1)
        .map_err(|e| CandleError::Inference(format!("Argmax failed: {}", e)))?
        .to_scalar::<u32>()
        .map_err(|e| CandleError::Inference(format!("Scalar conversion failed: {}", e)))
}
