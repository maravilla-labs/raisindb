//! Does an image written the way a JS function writes it actually reach the
//! model?
//!
//! # Why this test is live rather than a serialization assertion
//!
//! The failure this replaces was not a wrong byte on the wire. It was that
//! `ContentPart` accepted exactly one spelling of an image — the one no client
//! emits — while three others were parse errors and a fourth, the `data:` URL,
//! parsed into a variant `Message::first_image()` returns `None` for. So the
//! request succeeded, the model answered, and the answer described an image it
//! had never been shown. Nothing anywhere logged a thing.
//!
//! A unit test over `serde_json::from_str` (there are several, in
//! `types::tests`) proves the parse. Only an actual model can prove the last
//! link: that the bytes arrived and were LOOKED AT. So this asks a vision model
//! to name what is in a generated image, and — the part that makes it evidence
//! rather than a coin flip — asks the same question of a SECOND, different
//! image and requires the two answers to differ in the expected direction. A
//! model that receives no image at all will happily produce a plausible
//! sentence; it cannot produce "red square" for one and "blue circle" for the
//! other.
//!
//! Ignored by default: it needs a vision-capable model served by a local Ollama.
//!
//! ```bash
//! ollama pull gemma3:latest          # or any vision model
//! RAISIN_VISION_MODEL=gemma3:latest \
//!   cargo test -p raisin-ai --test all vision_integration -- --ignored --nocapture
//! ```

use raisin_ai::provider::AIProviderTrait;
use raisin_ai::types::CompletionRequest;
use raisin_ai::OllamaProvider;

/// NOTE the `/api`. `OllamaProvider` appends only the endpoint (`/chat`,
/// `/embed`) to its `base_url`, matching `OLLAMA_DEFAULT_BASE`; passing the
/// bare host gets a 404 from Ollama's router, which looks exactly like "the
/// model is missing".
const OLLAMA_URL: &str = "http://127.0.0.1:11434/api";
const OLLAMA_ROOT: &str = "http://127.0.0.1:11434";

fn model() -> String {
    std::env::var("RAISIN_VISION_MODEL").unwrap_or_else(|_| "gemma4:latest".to_string())
}

/// A tiny PNG, generated rather than committed, so the test carries no binary
/// fixture and the two images differ in exactly one obvious way.
fn png(shape: Shape) -> Vec<u8> {
    const W: u32 = 128;
    const H: u32 = 128;

    let img = image::RgbImage::from_fn(W, H, |x, y| match shape {
        Shape::RedSquare => {
            if (24..104).contains(&x) && (24..104).contains(&y) {
                image::Rgb([220, 20, 20])
            } else {
                image::Rgb([255, 255, 255])
            }
        }
        Shape::BlueCircle => {
            let dx = x as i32 - 64;
            let dy = y as i32 - 64;
            if dx * dx + dy * dy < 44 * 44 {
                image::Rgb([20, 40, 220])
            } else {
                image::Rgb([255, 255, 255])
            }
        }
    });

    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(img)
        .write_to(&mut out, image::ImageFormat::Png)
        .expect("PNG encode");
    out.into_inner()
}

#[derive(Clone, Copy)]
enum Shape {
    RedSquare,
    BlueCircle,
}

fn b64(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Build the request EXACTLY as a JS function does: as JSON, through
/// `serde_json`, in the OpenAI content-part spelling. Constructing a
/// `CompletionRequest` in Rust would skip the deserializer, which is where the
/// bug lived.
fn request_from_js_shaped_json(prompt: &str, image_png: &[u8]) -> CompletionRequest {
    let json = serde_json::json!({
        "model": model(),
        "temperature": 0.0,
        "messages": [{
            "role": "user",
            "content": [
                { "type": "text", "text": prompt },
                { "type": "image_url", "image_url": {
                    "url": format!("data:image/png;base64,{}", b64(image_png))
                }}
            ]
        }]
    });
    serde_json::from_value(json).expect("the JS-shaped request must deserialize")
}

async fn ollama_reachable() -> bool {
    reqwest::Client::new()
        .get(format!("{OLLAMA_ROOT}/api/tags"))
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .await
        .is_ok()
}

#[tokio::test]
#[ignore = "needs a local Ollama with a vision-capable model"]
async fn an_image_written_the_way_js_writes_it_reaches_the_model() {
    if !ollama_reachable().await {
        eprintln!("SKIP: no Ollama at {OLLAMA_ROOT}");
        return;
    }

    let provider = OllamaProvider::with_base_url(OLLAMA_URL);
    let prompt = "Name the single shape and its colour. Answer with two words only.";

    let red = provider
        .complete(request_from_js_shaped_json(prompt, &png(Shape::RedSquare)))
        .await
        .expect("completion must succeed");
    let blue = provider
        .complete(request_from_js_shaped_json(prompt, &png(Shape::BlueCircle)))
        .await
        .expect("completion must succeed");

    let red_answer = red.message.content.to_lowercase();
    let blue_answer = blue.message.content.to_lowercase();
    eprintln!("red  -> {red_answer}");
    eprintln!("blue -> {blue_answer}");

    // The DISCRIMINATION is the evidence. A model shown no image still answers;
    // it does not answer differently for two images it never saw.
    assert!(
        red_answer.contains("red") && red_answer.contains("square"),
        "the red square must be described as a red square, got: {red_answer}"
    );
    assert!(
        blue_answer.contains("blue") && blue_answer.contains("circle"),
        "the blue circle must be described as a blue circle, got: {blue_answer}"
    );
}

/// A remote image URL is REFUSED, not quietly dropped.
///
/// Ollama will not fetch a URL, and fetching it here would mean the server
/// issuing an arbitrary outbound request on behalf of tenant content — exactly
/// what `raisin.http.fetch`'s egress policy exists to stop. The only honest
/// answers are "refuse" and "fetch"; the one that must never come back is a
/// successful completion that saw nothing.
#[tokio::test]
#[ignore = "needs a local Ollama"]
async fn a_remote_image_url_is_refused_rather_than_dropped() {
    if !ollama_reachable().await {
        eprintln!("SKIP: no Ollama at {OLLAMA_ROOT}");
        return;
    }

    let json = serde_json::json!({
        "model": model(),
        "messages": [{
            "role": "user",
            "content": [
                { "type": "text", "text": "describe this" },
                { "type": "image_url", "image_url": { "url": "https://example.test/cat.jpg" }}
            ]
        }]
    });
    let request: CompletionRequest = serde_json::from_value(json).unwrap();

    let err = OllamaProvider::with_base_url(OLLAMA_URL)
        .complete(request)
        .await
        .expect_err("a URL Ollama cannot fetch must be an error, not a silent text-only call");
    let msg = err.to_string();
    assert!(
        msg.contains("remote image URL"),
        "the error must say what was wrong: {msg}"
    );
    assert!(
        msg.contains("data:image"),
        "and must say what to send instead: {msg}"
    );
}
