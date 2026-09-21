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
