mod add_unknown_variant;

use proc_macro::TokenStream;

#[proc_macro_attribute]
pub fn add_unknown_variant(args: TokenStream, input: TokenStream) -> TokenStream {
    add_unknown_variant::add_unknown_variant(args, input)
}