//! Handler registration and name routing.
//!
//! ONE artifact carries N handlers. The WIT export is fixed
//! (`handler(name, input)`); the handler *name* is data, selected by the
//! Function node's `entry_file` suffix (`main.wasm:shout` -> `"shout"`, a bare
//! `main.wasm` -> `"default"`). The host NEVER validates that name against a
//! list — the guest owns its handler namespace, so answering an unknown name
//! is this module's job.

/// One registered handler: its name and its JSON-in / JSON-out entry point.
///
/// Built by [`crate::export!`] from the `#[handler]`-annotated functions;
/// there is no reason to construct one by hand.
pub struct Handler {
    /// The name this handler answers to (`"default"` unless one was given).
    pub name: &'static str,
    /// JSON input -> JSON output, or a failure message.
    pub invoke: fn(&str) -> core::result::Result<String, String>,
}

/// Route `name` through `table`.
///
/// An unknown name is an `Err` listing every registered name, so a typo in an
/// `entry_file` is a legible failure rather than a silent no-op.
pub fn dispatch(
    table: &[Handler],
    name: &str,
    input: &str,
) -> core::result::Result<String, String> {
    match table.iter().find(|h| h.name == name) {
        Some(h) => (h.invoke)(input),
        None => Err(format!(
            "unknown handler '{name}'; registered: {}",
            registered_names(table).join(", ")
        )),
    }
}

/// The names in `table`, in declaration order.
pub fn registered_names(table: &[Handler]) -> Vec<&'static str> {
    table.iter().map(|h| h.name).collect()
}
