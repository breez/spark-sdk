use proc_macro::TokenStream;
use quote::quote;
use syn::Data;
use syn::DeriveInput;
use syn::parse_macro_input;

pub fn add_unknown_variant(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as DeriveInput);

    // Extract the enum's name, visibility, and attributes
    let name = &input.ident;
    let vis = &input.vis;
    let attrs = &input.attrs;

    // Handle generics properly
    let generics = &input.generics;
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    // Make sure we're working with an enum
    let Data::Enum(data_enum) = &input.data else {
        return quote! {
            compile_error!("add_unknown_variant can only be used with enums");
        }
        .into();
    };

    // Get all the variants from the original enum
    let variants = &data_enum.variants;

    // Check if Unknown variant already exists
    let unknown_exists = variants.iter().any(|variant| variant.ident == "Unknown");

    // Generate code for the new enum with the added Unknown variant (if needed)
    let expanded = if unknown_exists {
        // If Unknown already exists, just output the original enum and implement Default
        quote! {
            // Re-define the enum with all original attributes
            #(#attrs)*
            #vis enum #name #impl_generics #where_clause {
                // Include all the original variants (including the existing Unknown)
                #variants
            }

            // Implement Default to return the Unknown variant
            impl #impl_generics Default for #name #ty_generics #where_clause {
                fn default() -> Self {
                    Self::Unknown
                }
            }
        }
    } else {
        // If Unknown doesn't exist, add it
        quote! {
            // Re-define the enum with all original attributes
            #(#attrs)*
            #vis enum #name #impl_generics #where_clause {
                // Include all the original variants
                #variants
                /// An unknown variant, automatically added by the `add_unknown_variant` macro.
                ///
                /// This variant is used as the default value when `Default` is implemented.
                Unknown,
            }

            // Implement Default to return the Unknown variant
            impl #impl_generics Default for #name #ty_generics #where_clause {
                fn default() -> Self {
                    Self::Unknown
                }
            }
        }
    };

    expanded.into()
}
