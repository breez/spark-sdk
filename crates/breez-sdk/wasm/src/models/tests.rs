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

// Every file that mirrors SDK types for JS, since a tagged enum in one reaches
// structs in the others.
const MIRRORED: &[&str] = &[
    include_str!("mod.rs"),
    include_str!("chain_service.rs"),
    include_str!("issuer.rs"),
    include_str!("rest_client.rs"),
    include_str!("../passkey.rs"),
    include_str!("../sdk_builder.rs"),
    include_str!("../sdk_context.rs"),
    include_str!("../signer.rs"),
    include_str!("../turnkey.rs"),
];

// The rule behind the tests above, checked across every model. The models
// macro tags each enum that carries data, and a flattened field buffers the
// same way, so a u128 anywhere below either must use `serde_u128_as_string`
// or `serde_option_u128_as_string`.
#[wasm_bindgen_test]
fn buffered_models_hold_no_bigint() {
    let mut mirrored = Mirrored::default();
    for source in MIRRORED {
        mirrored.visit_file(&syn::parse_file(source).unwrap());
    }
    let Mirrored {
        fields,
        mut pending,
    } = mirrored;

    for name in OUTPUT_ONLY {
        assert!(
            fields.contains_key(*name),
            "OUTPUT_ONLY names no type: {name}"
        );
    }

    let mut seen = HashSet::new();
    let mut bigints = Vec::new();
    while let Some(name) = pending.pop() {
        if !seen.insert(name.clone()) {
            continue;
        }
        // A flattened field names a type, not a model, so it can be missing.
        let Some(type_fields) = fields.get(&name) else {
            continue;
        };
        for field in type_fields {
            let mut idents = Idents::default();
            idents.visit_type(&field.ty);
            if !serde_attr_has(field, "u128_as_string") && idents.0.iter().any(|i| i == "u128") {
                let field_name = field.ident.as_ref().map(ToString::to_string);
                bigints.push(format!("{name}.{}", field_name.unwrap_or_default()));
            }
            pending.extend(idents.0.into_iter().filter(|i| fields.contains_key(i)));
        }
    }
    assert!(
        bigints.is_empty(),
        "u128 below a tagged enum or a flattened field can't be read back from JS: \
         use serde_u128_as_string, \
         or list the enum in OUTPUT_ONLY if JS never passes it back. {bigints:?}"
    );
}

// Fields by type name, and where the walk starts: tagged enums and anything
// flattened. Visiting rather than reading `file.items` also reaches types
// declared inside a module.
#[derive(Default)]
struct Mirrored {
    fields: HashMap<String, Vec<syn::Field>>,
    pending: Vec<String>,
}

impl Mirrored {
    fn record(&mut self, name: String, fields: Vec<syn::Field>) {
        for field in &fields {
            if serde_attr_has(field, "flatten") {
                let mut idents = Idents::default();
                idents.visit_type(&field.ty);
                self.pending.extend(idents.0);
            }
        }
        self.fields.insert(name, fields);
    }
}

impl Visit<'_> for Mirrored {
    fn visit_item_struct(&mut self, item: &syn::ItemStruct) {
        let fields = item.fields.iter().cloned().collect();
        self.record(item.ident.to_string(), fields);
    }

    fn visit_item_enum(&mut self, item: &syn::ItemEnum) {
        let name = item.ident.to_string();
        if item.variants.iter().any(|v| !v.fields.is_empty())
            && !OUTPUT_ONLY.contains(&name.as_str())
        {
            self.pending.push(name.clone());
        }
        let variant_fields = item.variants.iter().flat_map(|v| v.fields.iter().cloned());
        self.record(name, variant_fields.collect());
    }
}

fn serde_attr_has(field: &syn::Field, needle: &str) -> bool {
    field.attrs.iter().any(|attr| match &attr.meta {
        syn::Meta::List(list) => list.tokens.to_string().contains(needle),
        _ => false,
    })
}

#[derive(Default)]
struct Idents(Vec<String>);

impl Visit<'_> for Idents {
    fn visit_ident(&mut self, ident: &syn::Ident) {
        self.0.push(ident.to_string());
    }
}
