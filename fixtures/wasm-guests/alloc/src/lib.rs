//! `alloc.wasm` — grows linear memory in 16 MiB chunks until the store's
//! `ResourceLimiter` refuses. The refusal makes the guest allocator abort,
//! which traps; the runtime maps that to `MEMORY_LIMIT` because the limiter
//! recorded a denied growth.

wit_bindgen::generate!({
    path: "../../../crates/raisin-functions/wit",
    world: "function",
});

struct Component;

const CHUNK: usize = 16 * 1024 * 1024;

impl Guest for Component {
    fn handler(_name: String, _input: String) -> Result<String, String> {
        let mut held: Vec<Vec<u8>> = Vec::new();
        for _ in 0..64 {
            let mut chunk = vec![0u8; CHUNK];
            // Touch every page so the allocation is actually backed.
            for i in (0..CHUNK).step_by(4096) {
                chunk[i] = 1;
            }
            held.push(chunk);
            std::hint::black_box(&held);
        }
        Ok(format!(r#"{{"chunks":{}}}"#, held.len()))
    }
}

export!(Component);
