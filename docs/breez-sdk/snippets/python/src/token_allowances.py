import logging
from breez_sdk_spark import (
    BreezSdk,
    CreateTokenAllowanceRequest,
    ListTokenAllowancesRequest,
    PreparePullPaymentRequest,
    PullPaymentRequest,
    PullReceiver,
    RevokeTokenAllowanceRequest,
    TokenAllowanceLimit,
    TokenAllowanceRole,
)


async def create_token_allowance(sdk: BreezSdk):
    # ANCHOR: create-token-allowance
    try:
        response = await sdk.create_token_allowance(
            request=CreateTokenAllowanceRequest(
                spender_public_key="<spender identity public key>",
                token_identifier="<token identifier>",
                max_per_payment=TokenAllowanceLimit.AMOUNT(amount=5_000_000),
                max_total=TokenAllowanceLimit.AMOUNT(amount=100_000_000),
                expiry_time=1_798_761_600,
                allowed_recipients=[],
            )
        )
        logging.debug(f"Allowance id: {response.allowance.id}")
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: create-token-allowance


async def list_token_allowances(sdk: BreezSdk):
    # ANCHOR: list-token-allowances
    try:
        response = await sdk.list_token_allowances(
            request=ListTokenAllowancesRequest(
                role=TokenAllowanceRole.OWNER,
                counterparty_public_key=None,
                token_identifier=None,
                include_inactive=False,
                offset=None,
                limit=None,
            )
        )
        for allowance in response.allowances:
            logging.debug(f"{allowance.id}: spent {allowance.spent_amount}")
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: list-token-allowances


async def revoke_token_allowance(sdk: BreezSdk):
    # ANCHOR: revoke-token-allowance
    try:
        await sdk.revoke_token_allowance(
            request=RevokeTokenAllowanceRequest(allowance_id="<allowance id>")
        )
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: revoke-token-allowance


async def pull_payment(sdk: BreezSdk):
    # ANCHOR: pull-payment
    try:
        prepare_response = await sdk.prepare_pull_payment(
            request=PreparePullPaymentRequest(
                payer_public_key="<payer identity public key>",
                token_identifier="<token identifier>",
                receivers=[PullReceiver(amount=5_000_000, receiver_public_key=None)],
            )
        )
        logging.debug(f"Pulling {prepare_response.amount}")

        response = await sdk.pull_payment(
            request=PullPaymentRequest(prepare_response=prepare_response)
        )
        logging.debug(f"Pull transaction: {response.tx_hash}")
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: pull-payment
