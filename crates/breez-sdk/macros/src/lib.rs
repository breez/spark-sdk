mod async_trait;
mod wasm_bindgen;

use proc_macro::TokenStream;

/// Attribute macro switch `async_trait` usage depending on WASM target
#[proc_macro_attribute]
pub fn async_trait(args: TokenStream, input: TokenStream) -> TokenStream {
    async_trait::async_trait(args, input)
}

/// Attribute macro to mirror the external struct/enum in WASM
///
/// ```rust,ignore
/// #[sdk_macros::extern_wasm_bindgen(sdk_common::prelude::RouteHint)]
/// pub struct RouteHint {
///     pub hops: Vec<RouteHintHop>,
/// }
/// ```
/// Generates in WASM typescript:
/// ```typescript
/// export interface RouteHint {
///     hops: RouteHintHop[];
/// }
/// ```
#[proc_macro_attribute]
pub fn extern_wasm_bindgen(args: TokenStream, input: TokenStream) -> TokenStream {
    wasm_bindgen::extern_wasm_bindgen(args, input)
}
