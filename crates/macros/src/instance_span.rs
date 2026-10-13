use quote::quote;
use syn::{ImplItem, ItemImpl, Visibility, parse_macro_input, parse_quote};

pub fn instance_span(
    _args: proc_macro::TokenStream,
    input: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let mut item = parse_macro_input!(input as ItemImpl);
    for impl_item in &mut item.items {
        let ImplItem::Fn(method) = impl_item else {
            continue;
        };
        if !matches!(method.vis, Visibility::Public(_)) || method.sig.receiver().is_none() {
            continue;
        }
        // The target must match `INSTANCE_SPAN_TARGET` in the SDK's logger, whose
        // filters keep it enabled. Error level keeps it enabled under any other
        // subscriber's filter that lets anything through.
        method.attrs.insert(
            0,
            parse_quote! {
                #[::tracing::instrument(
                    target = "breez_sdk_spark_instance",
                    level = "error",
                    skip_all,
                    fields(instance = %self.instance_id),
                )]
            },
        );
    }
    quote!(#item).into()
}
