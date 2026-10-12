use proc_macro2::TokenStream;
use quote::quote;

pub fn instance_span(
    _args: proc_macro::TokenStream,
    input: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let mut out = TokenStream::new();
    // The target must match `INSTANCE_SPAN_TARGET` in the SDK's logger, whose
    // filters keep it enabled. Error level keeps it enabled under any other
    // subscriber's filter that lets anything through.
    out.extend(quote! {
        #[::tracing::instrument(
            target = "breez_sdk_spark_instance",
            level = "error",
            skip_all,
            fields(instance = %self.instance_id),
        )]
    });
    out.extend(TokenStream::from(input));
    out.into()
}
