//! `sockets.wasm` — a well-formed function component that ALSO imports
//! `wasi:sockets`, which the host deliberately does not link. Instantiation
//! must fail naming that import; that is how egress stays `raisin.http.*` only.

wit_bindgen::generate!({
    path: "../../../crates/raisin-functions/wit",
    world: "function",
});

struct Component;

impl Guest for Component {
    fn handler(_name: String, _input: String) -> Result<String, String> {
        // The real `wasi:sockets` bindings, so the component's import name is
        // byte-for-byte the one an attacker would try to reach the network with.
        let network = wasi::sockets::instance_network::instance_network();
        let _ = wasi::sockets::ip_name_lookup::resolve_addresses(&network, "example.com");
        Ok("{}".to_string())
    }
}

export!(Component);
