# pylint: disable=duplicate-code
import logging
from breez_sdk_spark import (
    BreezSdk,
    CreateX402ChallengeRequest,
    PayX402Request,
    SettleX402Request,
    X402,
    X402Header,
    X402PaymentAmount,
    X402Request,
)


def get_x402(sdk: BreezSdk):
    # ANCHOR: get-x402
    x402 = sdk.get_x402()
    # ANCHOR_END: get-x402
    return x402


async def pay_http_request(x402: X402, payment_required_header: str):
    # ANCHOR: pay-http-request
    try:
        # The request that returned 402, exactly as it will be sent again
        request = X402Request.HTTP(
            method="GET",
            url="https://api.example.com/article/A",
            headers=[X402Header(name="accept", value="application/json")],
            body=None,
        )
        response = await x402.pay(
            request=PayX402Request(
                # The PAYMENT-REQUIRED header of the 402 response
                payment_required=payment_required_header,
                request=request,
                max_amount=X402PaymentAmount.LIGHTNING(amount_sats=100),
                conversion_options=None,
                completion_timeout_secs=None,
            )
        )

        if response.payment_signature is not None:
            # Send the request again with a PAYMENT-SIGNATURE header
            logging.debug(f"PAYMENT-SIGNATURE: {response.payment_signature}")
        else:
            # Still in flight: paying the same challenge again picks it up
            logging.debug(f"Payment status: {response.status}")
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: pay-http-request


async def pay_mcp_tool_call(x402: X402, structured_content: str):
    # ANCHOR: pay-mcp-tool-call
    try:
        request = X402Request.MCP(
            server="https://api.example.com/mcp",
            # The tools/call params, exactly as they will be sent again
            params='{"name":"get_article","arguments":{"article":"A"}}',
        )
        response = await x402.pay(
            request=PayX402Request(
                # The structuredContent of the tool result
                payment_required=structured_content,
                request=request,
                max_amount=X402PaymentAmount.LIGHTNING(amount_sats=100),
                conversion_options=None,
                completion_timeout_secs=None,
            )
        )

        if response.payment_signature is not None:
            # Call the tool again with this JSON in params._meta["x402/payment"]
            logging.debug(f"Payment: {response.payment_signature}")
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: pay-mcp-tool-call


async def create_challenge(x402: X402):
    # ANCHOR: create-challenge
    try:
        # The unpaid request, with its URL built from your public origin
        request = X402Request.HTTP(
            method="GET",
            url="https://api.example.com/article/A",
            headers=[X402Header(name="accept", value="application/json")],
            body=None,
        )
        response = await x402.create_challenge(
            request=CreateX402ChallengeRequest(
                request=request,
                # Headers that change what is served, so the payment covers them too
                bound_names=["accept"],
                amounts=[X402PaymentAmount.LIGHTNING(amount_sats=21)],
                max_timeout_secs=None,
                description="An article",
                mime_type="application/json",
            )
        )

        # Respond with status 402 and this PAYMENT-REQUIRED header
        logging.debug(f"PAYMENT-REQUIRED: {response.payment_required}")
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: create-challenge


async def settle_payment(x402: X402, payment_signature_header: str):
    # ANCHOR: settle-payment
    try:
        # The paid request, exactly as received
        request = X402Request.HTTP(
            method="GET",
            url="https://api.example.com/article/A",
            headers=[X402Header(name="accept", value="application/json")],
            body=None,
        )
        response = await x402.settle(
            request=SettleX402Request(
                # The PAYMENT-SIGNATURE header of the request
                payment=payment_signature_header,
                request=request,
                bound_names=["accept"],
            )
        )

        # Send the PAYMENT-RESPONSE header either way
        logging.debug(f"PAYMENT-RESPONSE: {response.payment_response}")
        if response.success:
            # Serve the resource
            pass
        else:
            # Respond with status 402 and a new challenge
            logging.debug(f"Payment refused: {response.error_reason}")
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: settle-payment
