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
    # amount is in source-asset base units
    # (e.g. USDC base units when source is USDC)
    amount = 1_000_000
    # Optionally set the destination Spark-side asset. None = auto: active
    # stable-balance token if the route supports it, otherwise BTC.
    optional_destination = None
    # Optionally set the maximum slippage in basis points (10 to 500)
    optional_max_slippage_bps = 100
    try:
        request = ReceivePaymentRequest(
            payment_method=ReceivePaymentMethod.CROSS_CHAIN(
                route=route,
                amount=amount,
                destination=optional_destination,
                max_slippage_bps=optional_max_slippage_bps,
            )
        )
        response = await sdk.receive_payment(request=request)
        deposit_address = response.payment_request
        logging.debug(
            f"Share this deposit address with the sender: {deposit_address}"
        )
        info = response.cross_chain_info
        if info is not None:
            logging.debug(f"Sender deposits: {info.deposit_amount}")
            denom = "USDB" if info.token_identifier else "BTC"
            logging.debug(
                f"Receiver gets ~{info.expected_received_amount} {denom}"
            )
            logging.debug(f"Quote expires at: {info.expires_at}")
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: cross-chain-receive
