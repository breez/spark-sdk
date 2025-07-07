mod derive_from;

use proc_macro::TokenStream;

#[proc_macro_attribute]
pub fn derive_from(attr: TokenStream, item: TokenStream) -> TokenStream {
    derive_from::derive_from(attr, item)
}
