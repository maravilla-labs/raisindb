//! Procedural macros for [`raisin-sdk`].
//!
//! Two macros, and the split between them is the whole registration design:
//!
//! - [`macro@handler`] marks one function as a named handler and generates the
//!   JSON envelope around it. It registers nothing by itself.
//! - [`export`] names the handlers a component exports, builds the dispatch
//!   table from them, and (on wasm targets only) emits the single
//!   `handler(name, input)` WIT export that routes on the name.
//!
//! Registration is an EXPLICIT, generated table rather than a link-time
//! inventory (`inventory`/`linkme`): `wasm32-wasip2` has no constructor
//! support this SDK is willing to rely on, and a distributed slice that
//! silently comes back empty would look exactly like "the guest registered no
//! handlers". A missing name in `export!` is a compile error instead.

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{
    parse::{Parse, ParseStream},
    parse_macro_input,
    punctuated::Punctuated,
    Ident, ItemFn, LitStr, Path, Token,
};

/// Marks a function as a RaisinDB handler and gives it a name.
///
/// ```ignore
/// #[raisin_sdk::handler]                    // registered as "default"
/// fn greet(input: Input) -> raisin_sdk::Result<Output> { .. }
///
/// #[raisin_sdk::handler(name = "shout")]    // registered as "shout"
/// fn shout(input: Input) -> raisin_sdk::Result<Output> { .. }
///
/// raisin_sdk::export!(greet, shout);
/// ```
///
/// The function keeps its signature and stays callable from Rust; the macro
/// only adds a hidden sibling module holding the handler's name and a
/// `&str -> Result<String, String>` JSON wrapper around it.
#[proc_macro_attribute]
pub fn handler(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as HandlerArgs);
    let func = parse_macro_input!(item as ItemFn);

    let fn_ident = func.sig.ident.clone();
    let name = args
        .name
        .map(|lit| lit.value())
        .unwrap_or_else(|| "default".to_string());
    if name.is_empty() {
        return syn::Error::new_spanned(&fn_ident, "handler name must not be empty")
            .to_compile_error()
            .into();
    }
    let module = format_ident!("__raisin_handler_{}", fn_ident);
    let doc = format!("Registration shim for the `{name}` handler.");

    quote! {
        #func

        #[doc = #doc]
        #[doc(hidden)]
        #[allow(non_snake_case)]
        pub mod #module {
            /// The name this handler is registered under.
            pub const NAME: &str = #name;

            /// JSON-in / JSON-out wrapper the dispatch table stores.
            pub fn invoke(
                input: &str,
            ) -> ::core::result::Result<::std::string::String, ::std::string::String> {
                ::raisin_sdk::__private::invoke_typed(input, super::#fn_ident)
            }
        }
    }
    .into()
}

/// Declares the handlers this component exports and emits its single WIT export.
///
/// Call once, at the crate root, listing every `#[handler]` function:
///
/// ```ignore
/// raisin_sdk::export!(greet, shout);
/// ```
///
/// It generates `raisin_dispatch(name, input)` — usable from native `cargo
/// test` — and, on wasm targets, the `handler(name, input)` export that routes
/// on the name and answers an unknown one with an `Err` listing what was
/// registered.
#[proc_macro]
pub fn export(item: TokenStream) -> TokenStream {
    let list = parse_macro_input!(item as ExportList);
    if list.handlers.is_empty() {
        return syn::Error::new(
            proc_macro2::Span::call_site(),
            "export! needs at least one handler; a component with no handlers can answer nothing",
        )
        .to_compile_error()
        .into();
    }

    let entries = list.handlers.iter().map(|path| {
        let mut module_path = path.clone();
        let last = module_path.segments.last_mut().expect("non-empty path");
        last.ident = format_ident!("__raisin_handler_{}", last.ident);
        quote! {
            ::raisin_sdk::Handler {
                name: #module_path::NAME,
                invoke: #module_path::invoke,
            }
        }
    });

    quote! {
        /// Every handler this component registered, in declaration order.
        #[doc(hidden)]
        pub static RAISIN_HANDLERS: &[::raisin_sdk::Handler] = &[#(#entries),*];

        /// Route `name` to a registered handler.
        ///
        /// An unknown name is an `Err` naming the registered set — the guest
        /// owns its handler namespace, so this is where that answer comes from.
        pub fn raisin_dispatch(
            name: &str,
            input: &str,
        ) -> ::core::result::Result<::std::string::String, ::std::string::String> {
            ::raisin_sdk::dispatch(RAISIN_HANDLERS, name, input)
        }

        #[cfg(target_family = "wasm")]
        #[doc(hidden)]
        struct RaisinComponent;

        #[cfg(target_family = "wasm")]
        impl ::raisin_sdk::bindings::Guest for RaisinComponent {
            fn handler(
                name: ::std::string::String,
                input: ::std::string::String,
            ) -> ::core::result::Result<::std::string::String, ::std::string::String> {
                raisin_dispatch(&name, &input)
            }
        }

        #[cfg(target_family = "wasm")]
        ::raisin_sdk::bindings::export_component!(RaisinComponent);
    }
    .into()
}

/// `#[handler]` / `#[handler(name = "...")]`.
struct HandlerArgs {
    name: Option<LitStr>,
}

impl Parse for HandlerArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        if input.is_empty() {
            return Ok(Self { name: None });
        }
        let key: Ident = input.parse()?;
        if key != "name" {
            return Err(syn::Error::new_spanned(
                key,
                "expected `name = \"...\"` (the only supported argument)",
            ));
        }
        input.parse::<Token![=]>()?;
        let name: LitStr = input.parse()?;
        Ok(Self { name: Some(name) })
    }
}

/// `export!(a, b, c)`.
struct ExportList {
    handlers: Punctuated<Path, Token![,]>,
}

impl Parse for ExportList {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        Ok(Self {
            handlers: Punctuated::parse_terminated(input)?,
        })
    }
}
