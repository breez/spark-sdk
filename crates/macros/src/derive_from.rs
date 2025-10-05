use proc_macro::TokenStream;
use quote::quote;
use syn::{
    DeriveInput, GenericArgument, Ident, PathArguments, Result, Type,
    parse::{Parse, ParseStream},
    parse_macro_input,
};

struct FromAttr {
    source_type: Ident,
}

impl Parse for FromAttr {
    fn parse(input: ParseStream) -> Result<Self> {
        let source_type: Ident = input.parse()?;
        Ok(FromAttr { source_type })
    }
}

pub fn derive_from(attr: TokenStream, item: TokenStream) -> TokenStream {
    // Parse the attribute to get the source type name
    let attr = parse_macro_input!(attr as FromAttr);
    let source_type = attr.source_type;

    // Parse the annotated item
    let input = parse_macro_input!(item as DeriveInput);
    let target_name = &input.ident;

    // Generate implementation based on type
    let implementation = match &input.data {
        syn::Data::Struct(data_struct) => {
            generate_struct_impl(source_type, target_name, data_struct)
        }
        _ => {
            return syn::Error::new_spanned(
                input.ident,
                "derive_from can only be used with structs",
            )
            .to_compile_error()
            .into();
        }
    };

    // Combine the original item with the implementation
    let expanded = quote! {
        #input

        #implementation
    };

    expanded.into()
}

/// Check if a type is an Option<T>
fn is_option_type(ty: &Type) -> bool {
    if let Type::Path(type_path) = ty
        && type_path.path.segments.len() == 1
    {
        return type_path.path.segments[0].ident == "Option";
    }
    false
}

/// Check if a type is a Vec<T>
fn is_vec_type(ty: &Type) -> bool {
    if let Type::Path(type_path) = ty
        && type_path.path.segments.len() == 1
    {
        return type_path.path.segments[0].ident == "Vec";
    }
    false
}

fn is_option_vec_type(ty: &Type) -> bool {
    if let Type::Path(type_path) = ty
        && type_path.path.segments.len() == 1
        && type_path.path.segments[0].ident == "Option"
        && let PathArguments::AngleBracketed(angle_bracketed_args) =
            &type_path.path.segments[0].arguments
        && angle_bracketed_args.args.len() == 1
        && let GenericArgument::Type(inner_type) = &angle_bracketed_args.args[0]
        && is_vec_type(inner_type)
    {
        return true;
    }

    false
}

fn generate_struct_impl(
    source_type: Ident,
    target_name: &Ident,
    data_struct: &syn::DataStruct,
) -> proc_macro2::TokenStream {
    match &data_struct.fields {
        syn::Fields::Named(fields) => {
            let field_conversions = fields.named.iter().map(|field| {
                let field_name = &field.ident;

                // Handle different container types
                if is_option_vec_type(&field.ty) {
                    quote! { #field_name: source.#field_name.map(|v| v.into_iter().map(Into::into).collect()) }
                } else if is_option_type(&field.ty) {
                    quote! { #field_name: source.#field_name.map(Into::into) }
                } else if is_vec_type(&field.ty) {
                    quote! { #field_name: source.#field_name.into_iter().map(Into::into).collect() }
                } else {
                    quote! { #field_name: source.#field_name.into() }
                }
            });

            quote! {
                impl From<#source_type> for #target_name {
                    fn from(source: #source_type) -> Self {
                        Self {
                            #(#field_conversions),*
                        }
                    }
                }
            }
        }
        syn::Fields::Unnamed(fields) => {
            let field_indices = 0..fields.unnamed.len();
            let field_conversions = field_indices.map(|i| {
                let index = syn::Index::from(i);
                let field = &fields.unnamed[i];

                if is_option_type(&field.ty) {
                    quote! { source.#index.map(Into::into) }
                } else if is_vec_type(&field.ty) {
                    quote! { source.#index.into_iter().map(Into::into).collect() }
                } else {
                    quote! { source.#index.into() }
                }
            });

            quote! {
                impl From<#source_type> for #target_name {
                    fn from(source: #source_type) -> Self {
                        Self(
                            #(#field_conversions),*
                        )
                    }
                }
            }
        }
        syn::Fields::Unit => {
            quote! {
                impl From<#source_type> for #target_name {
                    fn from(_source: #source_type) -> Self {
                        Self
                    }
                }
            }
        }
    }
}
