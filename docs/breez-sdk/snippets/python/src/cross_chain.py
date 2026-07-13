import logging
from breez_sdk_spark import (
    BreezSdk,
    CrossChainAddressDetails,
    CrossChainRouteFilter,
    CrossChainRoutePair,
    InputType,
    PaymentRequest,
    PrepareSendPaymentRequest,
    PrepareSendPaymentResponse,
    ReceivePaymentMethod,
    ReceivePaymentRequest,
    SendPaymentMethod,
    SendPaymentRequest,
)


async def get_cross_chain_routes(sdk: BreezSdk):
    # ANCHOR: cross-chain-get-routes
    input_str = "<recipient address>"
    try:
        parsed = await sdk.parse(input=input_str)
        if not isinstance(parsed, InputType.CROSS_CHAIN_ADDRESS):
            raise ValueError("Not a cross-chain address")
        address_details = parsed[0]

        routes = await sdk.get_cross_chain_routes(
            filter=CrossChainRouteFilter.SEND(address_details=address_details)
        )

        for route in routes:
            logging.debug(
                f"Route via {route.provider}: {route.chain}/{route.asset}"
            )
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: cross-chain-get-routes


async def prepare_send_payment_cross_chain(
    sdk: BreezSdk,
    address_details: CrossChainAddressDetails,
    route: CrossChainRoutePair,
):
    # ANCHOR: cross-chain-prepare
    # Optionally set the maximum slippage in basis points (10 to 500)
    optional_max_slippage_bps = 100
    try:
        request = PrepareSendPaymentRequest(
            payment_request=PaymentRequest.CROSS_CHAIN(
                address=address_details.address,
                route=route,
                max_slippage_bps=optional_max_slippage_bps,
                target_overpay_bps=None,
            ),
            amount=50_000,
            token_identifier=None,
            conversion_options=None,
            fee_policy=None,
        )
        prepare_response = await sdk.prepare_send_payment(request=request)

        if isinstance(
            prepare_response.payment_method, SendPaymentMethod.CROSS_CHAIN_ADDRESS
        ):
            method = prepare_response.payment_method
            logging.debug(f"Amount in: {method.amount_in}")
            logging.debug(f"Estimated out: {method.estimated_out}")
            logging.debug(f"Provider fee: {method.fee_amount}")
            logging.debug(f"Quote expires at: {method.expires_at}")
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: cross-chain-prepare


async def send_payment_cross_chain(
    sdk: BreezSdk,
    prepare_response: PrepareSendPaymentResponse,
):
    # ANCHOR: cross-chain-send
    # Only valid for sends with no token leg (see Retry safety).
    optional_idempotency_key = "<idempotency key uuid>"
    try:
        request = SendPaymentRequest(
            prepare_response=prepare_response,
            options=None,
            idempotency_key=optional_idempotency_key,
        )
        send_response = await sdk.send_payment(request=request)
        payment = send_response.payment
        logging.debug(f"Payment: {payment}")
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: cross-chain-send


async def get_cross_chain_receive_routes(sdk: BreezSdk):
    # ANCHOR: cross-chain-get-receive-routes
    try:
        routes = await sdk.get_cross_chain_routes(
            filter=CrossChainRouteFilter.RECEIVE(contract_address=None)
        )

        for route in routes:
            logging.debug(
                f"Route via {route.provider}: {route.chain}/{route.asset} -> Spark"
            )
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: cross-chain-get-receive-routes


async def receive_payment_cross_chain(sdk: BreezSdk, route: CrossChainRoutePair):
    # ANCHOR: cross-chain-receive
    # With the default FeesExcluded mode, amount is the receiver's net target
    # on Spark in destination-asset base units (sats for BTC, token base units
    # for USDB). The SDK pads the sender's deposit to cover fees + overpay.
    # With FeesIncluded, amount is the sender's deposit in source-asset units.
    amount = 1_000
    # Optionally set the destination Spark-side asset. None = auto: active
    # stable-balance token if the route supports it, otherwise BTC.
    optional_destination = None
    # Optionally set the maximum slippage in basis points (10 to 500)
    optional_max_slippage_bps = 100
    # Optionally override the overpay buffer (0 to 500 bps). Defaults to 15.
    optional_target_overpay_bps = None
    # Optionally override the fee mode. Defaults to FeesExcluded.
    optional_fee_mode = None
    try:
        request = ReceivePaymentRequest(
            payment_method=ReceivePaymentMethod.CROSS_CHAIN(
                route=route,
                amount=amount,
                destination=optional_destination,
                fee_mode=optional_fee_mode,
                max_slippage_bps=optional_max_slippage_bps,
                target_overpay_bps=optional_target_overpay_bps,
            )
        )
        response = await sdk.receive_payment(request=request)
        logging.debug(f"Payment request: {response.payment_request}")
        info = response.cross_chain_info
        if info is not None:
            logging.debug(f"Deposit address: {info.deposit_address}")
            logging.debug(f"Deposit amount: {info.deposit_amount}")
            denom = "USDB" if info.token_identifier else "BTC"
            logging.debug(
                f"Expected received: {info.expected_received_amount} {denom}"
            )
            logging.debug(f"Expires at: {info.expires_at}")
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: cross-chain-receive
