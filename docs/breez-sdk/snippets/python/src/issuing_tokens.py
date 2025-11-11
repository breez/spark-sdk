import logging
from breez_sdk_spark import (
    BreezSdk,
    BreezIssuerSdk,
    CreateIssuerTokenRequest,
    MintIssuerTokenRequest,
    FreezeIssuerTokenRequest,
    BurnIssuerTokenRequest,
    UnfreezeIssuerTokenRequest,
)


def get_issuer_sdk(sdk: BreezSdk):
    # ANCHOR: get-issuer-sdk
    issuer_sdk = sdk.get_issuer_sdk()
    # ANCHOR_END: get-issuer-sdk


async def create_token(issuer_sdk: BreezIssuerSdk):
    # ANCHOR: create-token
    try:
        request = CreateIssuerTokenRequest(
            name="My Token",
            ticker="MTK",
            decimals=6,
            is_freezable=False,
            max_supply=1_000_000,
        )
        token_metadata = await issuer_sdk.create_issuer_token(request)
        logging.debug(f"Token identifier: {token_metadata.identifier}")
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: create-token


async def mint_token(issuer_sdk: BreezIssuerSdk):
    # ANCHOR: mint-token
    try:
        request = MintIssuerTokenRequest(
            amount=1_000,
        )
        payment = await issuer_sdk.mint_issuer_token(request)
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: mint-token


async def burn_token(issuer_sdk: BreezIssuerSdk):
    # ANCHOR: burn-token
    try:
        request = BurnIssuerTokenRequest(
            amount=1_000,
        )
        payment = await issuer_sdk.burn_issuer_token(request)
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: burn-token


async def get_token_metadata(issuer_sdk: BreezIssuerSdk):
    # ANCHOR: get-token-metadata
    try:
        token_balance = await issuer_sdk.get_issuer_token_balance()
        logging.debug(f"Token balance: {token_balance.balance}")

        token_metadata = await issuer_sdk.get_issuer_token_metadata()
        logging.debug(f"Token ticker: {token_metadata.ticker}")
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: get-token-metadata


async def freeze_token(issuer_sdk: BreezIssuerSdk):
    # ANCHOR: freeze-token
    try:
        spark_address = "<spark address>"
        # Freeze the tokens held at the specified Spark address
        freeze_request = FreezeIssuerTokenRequest(
            address=spark_address,
        )
        freeze_response = await issuer_sdk.freeze_issuer_token(freeze_request)
        # Unfreeze the tokens held at the specified Spark address
        unfreeze_request = UnfreezeIssuerTokenRequest(
            address=spark_address,
        )
        unfreeze_response = await issuer_sdk.unfreeze_issuer_token(unfreeze_request)
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: freeze-token
