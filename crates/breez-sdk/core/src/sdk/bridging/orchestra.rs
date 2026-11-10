//! Bridge models from Orchestra's route listings and orders.

use flashnet::orchestra::{Order, OrderStatus, RouteAsset, RouteWithLimits};

use super::{BridgeRoutePair, BridgeRoutePairSide, BridgeStatus, GetBridgeStatusResponse};
use crate::{
    CrossChainProvider, DeliveryMethod,
    cross_chain::{delivery_method_from_wire, to_route_limits},
};

pub(super) fn to_bridge_route_pair(route: &RouteWithLimits) -> BridgeRoutePair {
    BridgeRoutePair {
        provider: CrossChainProvider::Orchestra,
        source: to_bridge_route_pair_side(&route.route.source),
        destination: to_bridge_route_pair_side(&route.route.destination),
        limits: to_route_limits(&route.limits),
    }
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
        assert_eq!(pair.limits.and_then(|l| l.min_usd_cents), Some(200));
    }
}
