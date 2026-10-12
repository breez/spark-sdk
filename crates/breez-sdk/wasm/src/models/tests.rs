use std::collections::{HashMap, HashSet};

use syn::visit::Visit;
use tsify_next::Tsify;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::wasm_bindgen_test;

use super::{
    CheckRecoverFundsRequest, CrossChainAcceptedAsset, CrossChainAddressDetails,
    CrossChainAddressFamily, CrossChainProvider, CrossChainRouteFilter, CrossChainRouteLimits,
    CrossChainRoutePair, DeliveryMethod, PaymentRequest, ReceivePaymentMethod, RecoverFundsLeaf,
    RecoverFundsResponse, RecoveryMethod, SparkAsset,
};

// Values the SDK returns must be accepted back, BigInts included, inside the
// tagged enums the models macro generates.

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

#[wasm_bindgen_test]
fn a_recovery_an_earlier_version_stored_does_not_parse() {
    let request = js_sys::JSON::parse(
        r#"{
            "recovery": {
                "recoverableValueSat": 100000,
                "totalFeeSat": 900,
                "cpfpFeeSat": 500,
                "fanoutFeeSat": 0,
                "sweepFeeSat": 400,
                "leaves": [{ "leafId": "leaf-1", "value": 100000 }],
                "transactions": [],
                "fundingInputs": [
                    { "type": "p2tr", "txid": "bb", "vout": 0, "value": 5000, "pubkey": "02ab" }
                ]
            }
        }"#,
    )
    .unwrap();

    assert!(CheckRecoverFundsRequest::from_js(request).is_err());
}

#[wasm_bindgen_test]
fn a_recovery_this_version_returned_is_read() {
    let request = CheckRecoverFundsRequest {
        recovery: RecoverFundsResponse {
            recoverable_value_sats: 100_000,
            total_fee_sats: 900,
            cooperative_fee_sats: 0,
            cpfp_fee_sats: 500,
            fanout_fee_sats: 0,
            sweep_fee_sats: 400,
            leaves: vec![RecoverFundsLeaf {
                leaf_id: "leaf-1".to_string(),
                value_sats: 100_000,
                method: RecoveryMethod::Unilateral,
            }],
            failed: Vec::new(),
            transactions: Vec::new(),
            funding_inputs: Vec::new(),
            fee_rate_sat_per_vbyte: 2,
            destination: "bcrt1qdestination".to_string(),
        },
    };
    let js = request.into_js().unwrap();

    let Ok(read) = CheckRecoverFundsRequest::from_js(js) else {
        panic!("expected the recovery to be read");
    };

    assert_eq!(read.recovery.leaves.len(), 1);
}

#[wasm_bindgen_test]
fn tagged_enums_read_bigints_beyond_u64() {
    let method = ReceivePaymentMethod::CrossChain {
        route: route_with_limits(),
        amount: u128::MAX - 1,
        destination: Some(SparkAsset::Token {
            token_identifier: "btkn1".to_string(),
        }),
        fee_mode: None,
        max_slippage_bps: None,
        target_overpay_bps: None,
    };
    let js = method.into_js().unwrap();
    let ReceivePaymentMethod::CrossChain {
        route,
        amount,
        destination: Some(SparkAsset::Token { token_identifier }),
        ..
    } = ReceivePaymentMethod::from_js(js).unwrap()
    else {
        panic!("expected a cross-chain method paying out a token");
    };
    assert_eq!(amount, u128::MAX - 1);
    assert_eq!(token_identifier, "btkn1");
    let limits = route.accepted_assets[0].limits.as_ref().unwrap();
    assert_eq!(limits.max_amount, Some(u128::MAX));

    let js = ReceivePaymentMethod::SparkAddress.into_js().unwrap();
    assert!(matches!(
        ReceivePaymentMethod::from_js(js).unwrap(),
        ReceivePaymentMethod::SparkAddress
    ));
}

#[wasm_bindgen_test]
fn tagged_enums_read_amounts_passed_as_strings() {
    let js = js_sys::eval(
        "({ amount: '340282366920938463463374607431768211455', expiryTime: 60, type: 'sparkInvoice' })",
    )
    .unwrap();
    let ReceivePaymentMethod::SparkInvoice {
        amount,
        expiry_time,
        ..
    } = ReceivePaymentMethod::from_js(js).unwrap()
    else {
        panic!("expected a Spark invoice method");
    };
    assert_eq!(amount, Some(u128::MAX));
    assert_eq!(expiry_time, Some(60));
}

#[wasm_bindgen_test]
fn tagged_enums_report_what_is_wrong() {
    let message = |source: &str| {
        let Err(error) = ReceivePaymentMethod::from_js(js_sys::eval(source).unwrap()) else {
            panic!("expected {source} to be rejected");
        };
        let error: js_sys::Error = wasm_bindgen::JsValue::from(error).dyn_into().unwrap();
        String::from(error.message())
    };
    assert!(message("({ type: 'nope' })").starts_with("unknown variant `nope`"));
    assert_eq!(message("({ amount: 1n })"), "missing field `type`");
    assert_eq!(
        message("({ type: 'crossChain', amount: 'lots' })"),
        "expected a u128 BigInt, got JsValue(\"lots\")"
    );
}

// Every file whose types cross to JS, since a type in one reaches structs in
// the others.
const MIRRORED: &[&str] = &[
    include_str!("mod.rs"),
    include_str!("chain_service.rs"),
    include_str!("issuer.rs"),
    include_str!("rest_client.rs"),
    include_str!("../passkey.rs"),
    include_str!("../sdk_builder.rs"),
    include_str!("../sdk_context.rs"),
    include_str!("../signer.rs"),
    include_str!("../token_store/mod.rs"),
    include_str!("../tree_store/mod.rs"),
    include_str!("../turnkey.rs"),
];

// Serde buffers a flattened field, and an enum tagged by hand rather than by
// the models macro, in a form that has no u128. A u128 below either can't be
// read back from JS.
#[wasm_bindgen_test]
fn buffered_models_hold_no_u128() {
    let mut mirrored = Mirrored::default();
    for source in MIRRORED {
        mirrored.visit_file(&syn::parse_file(source).unwrap());
    }
    let Mirrored {
        fields,
        mut pending,
    } = mirrored;

    let mut seen = HashSet::new();
    let mut u128s = Vec::new();
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
            if idents.0.iter().any(|i| i == "u128") {
                let field_name = field.ident.as_ref().map(ToString::to_string);
                u128s.push(format!("{name}.{}", field_name.unwrap_or_default()));
            }
            pending.extend(idents.0.into_iter().filter(|i| fields.contains_key(i)));
        }
    }
    assert!(
        u128s.is_empty(),
        "u128 below a flattened field or a hand-tagged enum can't be read back from JS: \
         mirror the enum with `extern_wasm_bindgen` instead. {u128s:?}"
    );
}

// Fields by type name, and where the walk starts: hand-tagged enums and
// anything flattened. Visiting rather than reading `file.items` also reaches
// types declared inside a module.
#[derive(Default)]
struct Mirrored {
    fields: HashMap<String, Vec<syn::Field>>,
    pending: Vec<String>,
}

impl Mirrored {
    fn record(&mut self, name: String, fields: Vec<syn::Field>) {
        for field in &fields {
            if serde_attr_has(&field.attrs, "flatten") {
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
        if serde_attr_has(&item.attrs, "tag") || serde_attr_has(&item.attrs, "untagged") {
            self.pending.push(name.clone());
        }
        let variant_fields = item.variants.iter().flat_map(|v| v.fields.iter().cloned());
        self.record(name, variant_fields.collect());
    }
}

fn serde_attr_has(attrs: &[syn::Attribute], needle: &str) -> bool {
    attrs.iter().any(|attr| match &attr.meta {
        syn::Meta::List(list) if list.path.is_ident("serde") => {
            list.tokens.to_string().contains(needle)
        }
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
