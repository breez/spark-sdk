use std::collections::{HashMap, HashSet};

use syn::visit::Visit;
use tsify_next::Tsify;
use wasm_bindgen_test::wasm_bindgen_test;

use super::{
    CrossChainAcceptedAsset, CrossChainAddressDetails, CrossChainAddressFamily, CrossChainProvider,
    CrossChainRouteFilter, CrossChainRouteLimits, CrossChainRoutePair, DeliveryMethod,
    PaymentRequest, SparkAsset,
};

// Values the SDK returns must be accepted back inside an internally tagged
// enum, which buffers its fields in a form that has no u128.

fn route_with_limits() -> CrossChainRoutePair {
    CrossChainRoutePair {
        provider: CrossChainProvider::Orchestra,
        chain: "solana".to_string(),
        chain_id: None,
        asset: "USDC".to_string(),
        contract_address: None,
        decimals: 6,
        exact_out_eligible: false,
        accepted_assets: vec![CrossChainAcceptedAsset {
            asset: SparkAsset::Bitcoin,
            limits: Some(CrossChainRouteLimits {
                min_amount: Some(1_200),
                max_amount: Some(u128::MAX),
                min_usd_cents: Some(80),
                max_usd_cents: None,
            }),
        }],
        delivery_methods: vec![DeliveryMethod::Spark],
    }
}

#[wasm_bindgen_test]
fn payment_request_accepts_a_route_with_amount_limits() {
    let request = PaymentRequest::CrossChain {
        address: "9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM".to_string(),
        route: route_with_limits(),
        max_slippage_bps: None,
        target_overpay_bps: None,
    };
    let js = request.into_js().unwrap();
    let PaymentRequest::CrossChain { route, .. } = PaymentRequest::from_js(js).unwrap() else {
        panic!("expected a cross-chain request");
    };
    let limits = route.accepted_assets[0].limits.as_ref().unwrap();
    assert_eq!(limits.min_amount, Some(1_200));
    assert_eq!(limits.max_amount, Some(u128::MAX));
}

#[wasm_bindgen_test]
fn route_filter_accepts_address_details_with_an_amount() {
    let filter = CrossChainRouteFilter::Send {
        address_details: CrossChainAddressDetails {
            address: "0x742d35Cc6634C0532925a3b844Bc454e4438f44e".to_string(),
            address_family: CrossChainAddressFamily::Evm,
            contract_address: None,
            chain_id: Some(1),
            amount: Some(5_000_000),
        },
    };
    let js = filter.into_js().unwrap();
    let CrossChainRouteFilter::Send { address_details } =
        CrossChainRouteFilter::from_js(js).unwrap()
    else {
        panic!("expected a send filter");
    };
    assert_eq!(address_details.amount, Some(5_000_000));
}

// Enums the SDK only hands to JS. Nothing reads them back, so they may hold a
// BigInt.
const OUTPUT_ONLY: &[&str] = &[
    "SdkEvent",
    "PublishSignedLnurlPayResponse",
    "PublishSignedTransferPackageResponse",
];

// The rule behind the tests above, checked across every model: the models
// macro tags each enum that carries data, so a u128 anywhere below one must use
// `serde_u128_as_string` or `serde_option_u128_as_string`.
#[wasm_bindgen_test]
fn tagged_enums_reach_no_bigint() {
    let file = syn::parse_file(include_str!("mod.rs")).unwrap();
    let mut fields: HashMap<String, Vec<syn::Field>> = HashMap::new();
    let mut pending = Vec::new();
    for item in file.items {
        match item {
            syn::Item::Struct(s) => {
                fields.insert(s.ident.to_string(), s.fields.into_iter().collect());
            }
            syn::Item::Enum(e) => {
                let name = e.ident.to_string();
                if e.variants.iter().any(|v| !v.fields.is_empty())
                    && !OUTPUT_ONLY.contains(&name.as_str())
                {
                    pending.push(name.clone());
                }
                let variant_fields = e.variants.into_iter().flat_map(|v| v.fields);
                fields.insert(name, variant_fields.collect());
            }
            _ => {}
        }
    }

    let mut seen = HashSet::new();
    let mut bigints = Vec::new();
    while let Some(name) = pending.pop() {
        if !seen.insert(name.clone()) {
            continue;
        }
        for field in &fields[&name] {
            let mut idents = Idents::default();
            idents.visit_type(&field.ty);
            let as_string = field.attrs.iter().any(|attr| match &attr.meta {
                syn::Meta::List(list) => list.tokens.to_string().contains("u128_as_string"),
                _ => false,
            });
            if !as_string && idents.0.iter().any(|ident| ident == "u128") {
                let field_name = field.ident.as_ref().map(ToString::to_string);
                bigints.push(format!("{name}.{}", field_name.unwrap_or_default()));
            }
            pending.extend(idents.0.into_iter().filter(|i| fields.contains_key(i)));
        }
    }
    assert!(
        bigints.is_empty(),
        "u128 below a tagged enum can't be read back from JS: use serde_u128_as_string, \
         or list the enum in OUTPUT_ONLY if JS never passes it back. {bigints:?}"
    );
}

#[derive(Default)]
struct Idents(Vec<String>);

impl Visit<'_> for Idents {
    fn visit_ident(&mut self, ident: &syn::Ident) {
        self.0.push(ident.to_string());
    }
}
