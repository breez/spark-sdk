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
        // subscriber's filter that lets anything through. Appended so it is the
        // outermost span: the last `#[instrument]` listed wraps the others, and a
        // method's own span must stay current for it to record its fields.
        method.attrs.push(parse_quote! {
            #[::tracing::instrument(
                target = "breez_sdk_spark_instance",
                level = "error",
                skip_all,
                fields(instance = %self.instance_id),
            )]
        });
    }
    quote!(#item).into()
}
