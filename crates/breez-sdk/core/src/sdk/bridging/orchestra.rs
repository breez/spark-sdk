//! Bridge models from Orchestra's route listings and orders.

use flashnet::orchestra::{Order, OrderStatus, RouteAsset, RouteLimits, RouteWithLimits};

use super::{
    BridgeRoutePair, BridgeRoutePairSide, BridgeStatus, GetBridgeStatusResponse, amount_side,
};
use crate::{
    CrossChainProvider, CrossChainRouteLimits, DeliveryMethod,
    cross_chain::{delivery_method_from_wire, to_route_limits},
};

/// Orchestra's constraint on the least an on-chain Bitcoin delivery must
/// reach the recipient with.
const BITCOIN_DELIVERY_MIN: &str = "bitcoin_l1_delivery_min";

pub(super) fn to_bridge_route_pair(route: &RouteWithLimits) -> BridgeRoutePair {
    let destination = to_bridge_route_pair_side(&route.route.destination);
    let limits = if delivery_method_from_wire(&destination.chain) == Some(DeliveryMethod::Bitcoin) {
        bitcoin_delivery_limits(&route.limits)
    } else {
        to_route_limits(&route.limits)
    };
    BridgeRoutePair {
        provider: CrossChainProvider::Orchestra,
        source: to_bridge_route_pair_side(&route.route.source),
        amount_side: amount_side(&destination.asset),
        destination,
        limits,
    }
}

/// The bounds of a route paying Bitcoin on-chain. Its minimum is the sats
/// that must reach the recipient, which follows the exit fee, in place of
/// the route's USD minimum, which sits far below it.
fn bitcoin_delivery_limits(limits: &RouteLimits) -> Option<CrossChainRouteLimits> {
    let floor = limits
        .constraints
        .iter()
        .find(|c| c.id == BITCOIN_DELIVERY_MIN && c.leg.as_deref() == Some("destination"))
        .and_then(|c| c.min_amount_smallest.as_deref()?.parse().ok());
    let listed = to_route_limits(limits);
    let bounds = CrossChainRouteLimits {
        min_amount: floor,
        max_amount: listed.as_ref().and_then(|l| l.max_amount),
        min_usd_cents: None,
        max_usd_cents: listed.and_then(|l| l.max_usd_cents),
    };
    (bounds.min_amount.is_some() || bounds.max_amount.is_some() || bounds.max_usd_cents.is_some())
        .then_some(bounds)
}

fn to_bridge_route_pair_side(side: &RouteAsset) -> BridgeRoutePairSide {
    BridgeRoutePairSide {
        chain: side.chain.clone(),
        chain_id: side.chain_id.clone(),
        asset: side.asset.clone(),
        contract_address: side.contract_address.clone(),
        decimals: side.decimals,
    }
}

/// Whether nobody has paid `order`, an onramp order. It exists from its
/// creation, and stays `processing` until paid. `/status` may leave out its
/// `sourceChain`, so the order's kind comes from how the bridge reads it.
pub(super) fn is_unpaid_onramp_order(order: &Order) -> bool {
    order.status == OrderStatus::Processing
}

/// The status of a bridge whose order is `order`, or `Created` before it has
/// one.
pub(super) fn bridge_status_response(order: Option<&Order>) -> GetBridgeStatusResponse {
    let Some(order) = order else {
        return GetBridgeStatusResponse {
            status: BridgeStatus::Created,
            provider_status: None,
            order_id: None,
            amount_in: None,
            amount_out: None,
            deposit_tx_hash: None,
            delivery_tx_hash: None,
            refund_tx_hash: None,
            error_code: None,
        };
    };
    let status = bridge_status(order.status);
    let completed = status == BridgeStatus::Completed;
    let amount = |value: &Option<String>| value.as_deref().and_then(|v| v.parse::<u128>().ok());
    // Before completion the delivery fields can describe an internal leg of
    // the order, e.g. the Spark transfer of a Lightning-funded deposit. The
    // Spark transfer is the delivery only to a Spark recipient.
    let to_spark = order
        .destination_chain
        .as_deref()
        .and_then(delivery_method_from_wire)
        == Some(DeliveryMethod::Spark);
    let delivery_tx_hash = completed
        .then(|| {
            order
                .destination_tx_hash
                .clone()
                .or_else(|| order.spark_tx_hash.clone().filter(|_| to_spark))
        })
        .flatten();
    GetBridgeStatusResponse {
        status,
        provider_status: provider_status(order.status),
        order_id: Some(order.id.clone()),
        amount_in: amount(&order.amount_in),
        amount_out: completed.then(|| amount(&order.amount_out)).flatten(),
        deposit_tx_hash: order.source_tx_hash.clone(),
        delivery_tx_hash,
        refund_tx_hash: order.refund_tx_hash.clone(),
        error_code: order.error_code.clone(),
    }
}

fn bridge_status(status: OrderStatus) -> BridgeStatus {
    match status {
        OrderStatus::Refunding => BridgeStatus::Refunding,
        // `FallbackDelivered` only follows a quote into an unlisted token,
        // which bridges never request.
        OrderStatus::Completed | OrderStatus::FallbackDelivered => BridgeStatus::Completed,
        OrderStatus::Refunded => BridgeStatus::Refunded,
        OrderStatus::Failed => BridgeStatus::Failed,
        OrderStatus::Expired => BridgeStatus::Expired,
        OrderStatus::Processing
        | OrderStatus::Confirming
        | OrderStatus::Bridging
        | OrderStatus::Swapping
        | OrderStatus::AwaitingApproval
        | OrderStatus::Delivering
        | OrderStatus::Unfulfilled
        | OrderStatus::Unknown => BridgeStatus::Pending,
    }
}

/// The provider's own name for `status`, e.g. `"bridging"`.
fn provider_status(status: OrderStatus) -> Option<String> {
    serde_json::to_value(status)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
}

#[cfg(test)]
mod tests {
    use macros::test_all;

    use super::super::BridgeAmountSide;
    use super::*;

    fn order(status: &str) -> Order {
        serde_json::from_value(serde_json::json!({
            "id": "ord_1",
            "status": status,
            "quoteId": "q_1",
            "sourceTxHash": "0xdeposit",
            "destinationTxHash": "0xdelivery",
            "amountIn": "10000000",
            "amountOut": "9950000",
            "refundTxHash": null,
            "errorCode": null,
            "createdAt": "2026-10-29T10:00:00Z",
            "updatedAt": "2026-10-29T10:05:00Z",
        }))
        .unwrap()
    }

    #[test_all]
    fn an_onramp_order_is_unpaid_while_processing() {
        assert!(is_unpaid_onramp_order(&order("processing")));
        assert!(!is_unpaid_onramp_order(&order("swapping")));
        assert!(!is_unpaid_onramp_order(&order("expired")));
    }

    #[test_all]
    fn bridge_status_response_is_created_without_an_order() {
        let response = bridge_status_response(None);
        assert_eq!(response.status, BridgeStatus::Created);
        assert_eq!(response.order_id, None);
        assert_eq!(response.provider_status, None);
    }

    #[test_all]
    fn bridge_status_response_maps_order_statuses() {
        for (wire, expected) in [
            ("processing", BridgeStatus::Pending),
            ("confirming", BridgeStatus::Pending),
            ("bridging", BridgeStatus::Pending),
            ("swapping", BridgeStatus::Pending),
            ("awaiting_approval", BridgeStatus::Pending),
            ("delivering", BridgeStatus::Pending),
            ("unfulfilled", BridgeStatus::Pending),
            ("something_new", BridgeStatus::Pending),
            ("refunding", BridgeStatus::Refunding),
            ("completed", BridgeStatus::Completed),
            ("fallback_delivered", BridgeStatus::Completed),
            ("refunded", BridgeStatus::Refunded),
            ("failed", BridgeStatus::Failed),
            ("expired", BridgeStatus::Expired),
        ] {
            let response = bridge_status_response(Some(&order(wire)));
            assert_eq!(response.status, expected, "{wire}");
        }
    }

    #[test_all]
    fn bridge_status_response_carries_the_order_details() {
        let response = bridge_status_response(Some(&order("completed")));
        assert_eq!(response.provider_status.as_deref(), Some("completed"));
        assert_eq!(response.order_id.as_deref(), Some("ord_1"));
        assert_eq!(response.amount_in, Some(10_000_000));
        assert_eq!(response.amount_out, Some(9_950_000));
        assert_eq!(response.deposit_tx_hash.as_deref(), Some("0xdeposit"));
        assert_eq!(response.delivery_tx_hash.as_deref(), Some("0xdelivery"));
    }

    #[test_all]
    fn bridge_status_response_names_the_delivery_once_completed() {
        let mut delivering = order("delivering");
        delivering.spark_tx_hash = Some("spark-leg".to_string());
        let response = bridge_status_response(Some(&delivering));
        assert_eq!(response.amount_out, None);
        assert_eq!(response.delivery_tx_hash, None);

        // A Spark recipient is paid by the Spark transfer.
        let mut to_spark = order("completed");
        to_spark.destination_chain = Some("spark".to_string());
        to_spark.destination_tx_hash = None;
        to_spark.spark_tx_hash = Some("spark-transfer".to_string());
        let response = bridge_status_response(Some(&to_spark));
        assert_eq!(response.delivery_tx_hash.as_deref(), Some("spark-transfer"));

        // Elsewhere a Spark transfer is an internal leg of the order.
        to_spark.destination_chain = Some("base".to_string());
        let response = bridge_status_response(Some(&to_spark));
        assert_eq!(response.delivery_tx_hash, None);
    }

    #[test_all]
    fn bridge_route_pairs_keep_both_sides_and_the_limits() {
        let route: RouteWithLimits = serde_json::from_value(serde_json::json!({
            "sourceChain": "tron",
            "sourceAsset": "USDT",
            "destinationChain": "base",
            "destinationAsset": "USDC",
            "source": {
                "chain": "tron", "asset": "USDT", "contractAddress": "TXYZtronUsdt",
                "decimals": 6, "chainId": null,
            },
            "destination": {
                "chain": "base", "asset": "USDC", "contractAddress": "0xUSDC",
                "decimals": 6, "chainId": "8453",
            },
            "limits": { "orderNotionalUsd": { "minCents": "200", "maxCents": "1200000000" } },
        }))
        .unwrap();
        let pair = to_bridge_route_pair(&route);
        assert_eq!(pair.provider, CrossChainProvider::Orchestra);
        assert_eq!(pair.source.chain, "tron");
        assert_eq!(
            pair.source.contract_address.as_deref(),
            Some("TXYZtronUsdt")
        );
        assert_eq!(pair.destination.chain, "base");
        assert_eq!(pair.destination.chain_id.as_deref(), Some("8453"));
        assert_eq!(pair.amount_side, BridgeAmountSide::Destination);
        assert_eq!(pair.limits.and_then(|l| l.min_usd_cents), Some(200));
    }

    #[test_all]
    fn a_native_coin_route_takes_the_amount_in_source_units() {
        let route: RouteWithLimits = serde_json::from_value(serde_json::json!({
            "sourceChain": "lightning",
            "sourceAsset": "BTC",
            "destinationChain": "base",
            "destinationAsset": "ETH",
            "source": {
                "chain": "lightning", "asset": "BTC", "contractAddress": null,
                "decimals": 8, "chainId": null,
            },
            "destination": {
                "chain": "base", "asset": "ETH", "contractAddress": null,
                "decimals": 18, "chainId": "8453",
            },
            "limits": {},
        }))
        .unwrap();
        let pair = to_bridge_route_pair(&route);
        assert_eq!(pair.destination.contract_address, None);
        assert_eq!(pair.amount_side, BridgeAmountSide::Source);
    }

    #[test_all]
    fn a_bitcoin_on_chain_route_lists_its_delivery_floor() {
        let route: RouteWithLimits = serde_json::from_value(serde_json::json!({
            "sourceChain": "lightning",
            "sourceAsset": "BTC",
            "destinationChain": "bitcoin",
            "destinationAsset": "BTC",
            "source": {
                "chain": "lightning", "asset": "BTC", "contractAddress": null,
                "decimals": 8, "chainId": null,
            },
            "destination": {
                "chain": "bitcoin", "asset": "BTC", "contractAddress": null,
                "decimals": 8, "chainId": null,
            },
            "limits": {
                "orderNotionalUsd": { "minCents": "80", "maxCents": "5240000" },
                "exactIn": { "supported": true, "requestAmount": {
                    "minAmountSmallest": null, "minUsdCents": "80", "maxUsdCents": "5230000",
                }},
                "constraints": [
                    { "id": "order_notional_usd", "leg": "route", "minUsdCents": "80" },
                    { "id": "bitcoin_l1_delivery_min", "leg": "destination",
                      "minAmountSmallest": "8530" },
                ],
            },
        }))
        .unwrap();
        let limits = to_bridge_route_pair(&route).limits.unwrap();
        assert_eq!(limits.min_amount, Some(8_530));
        assert_eq!(limits.min_usd_cents, None);
        assert_eq!(limits.max_usd_cents, Some(5_230_000));
    }

    #[test_all]
    fn an_on_chain_bitcoin_delivery_is_its_bitcoin_transaction() {
        let mut to_bitcoin = order("completed");
        to_bitcoin.destination_chain = Some("bitcoin".to_string());
        to_bitcoin.destination_tx_hash = Some("btc-txid".to_string());
        to_bitcoin.spark_tx_hash = Some("spark-leg".to_string());
        let response = bridge_status_response(Some(&to_bitcoin));
        assert_eq!(response.delivery_tx_hash.as_deref(), Some("btc-txid"));

        // The Spark transfer is an internal leg, never the delivery.
        to_bitcoin.destination_tx_hash = None;
        let response = bridge_status_response(Some(&to_bitcoin));
        assert_eq!(response.delivery_tx_hash, None);
    }
}
