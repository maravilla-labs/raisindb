//! `spin.wasm` — never returns. Proves epoch interruption cuts guest code off
//! at the configured deadline instead of hanging a worker forever.

wit_bindgen::generate!({
    path: "../../../crates/raisin-functions/wit",
    world: "function",
});

struct Component;

impl Guest for Component {
    fn handler(_name: String, _input: String) -> Result<String, String> {
        let mut n: u64 = 0;
        loop {
            n = n.wrapping_add(1);
            std::hint::black_box(n);
        }
    }
}

export!(Component);
