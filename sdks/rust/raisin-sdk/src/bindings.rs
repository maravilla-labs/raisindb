//! The generated WIT bindings — wasm targets only.
//!
//! `pub_export_macro` re-exports the world's export macro so a *user* crate can
//! export the component without invoking `wit_bindgen::generate!` itself:
//! [`crate::export!`] expands to `raisin_sdk::bindings::export_component!`.

// The bindings are machine-generated and wit-bindgen emits no item docs.
#![allow(missing_docs)]

wit_bindgen::generate!({
    path: "wit",
    world: "function",
    pub_export_macro: true,
    export_macro_name: "export_component",
    default_bindings_module: "::raisin_sdk::bindings",
});
