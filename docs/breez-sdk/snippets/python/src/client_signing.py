# pylint: disable=duplicate-code
import logging
from breez_sdk_spark import (
    BreezSdk,
    BuildTransferPackageOptions,
    BuildUnsignedLnurlPayPackageRequest,
    BuildUnsignedTransferPackageRequest,
    ExternalSparkSigner,
    LnurlPayResponse,
    OnchainConfirmationSpeed,
    Payment,
    PaymentRequest,
    PrepareLnurlPayResponse,
    PrepareSendPaymentRequest,
    PrepareSendPaymentResponse,
    PublishSignedLnurlPayPackageRequest,
    PublishSignedLnurlPayResponse,
    PublishSignedTransferPackageRequest,
    PublishSignedTransferPackageResponse,
    SignedTransferPackage,
    TransferSignature,
    TransferTarget,
    UnsignedTransferPackage,
)


async def sign_package(
    signer: ExternalSparkSigner, unsigned: UnsignedTransferPackage
) -> SignedTransferPackage:
    # ANCHOR: client-signing-sign-package
    if isinstance(unsigned, UnsignedTransferPackage.TRANSFER):
        # Show the user what they are approving before signing
        target = unsigned.target
        destination = ""
        if isinstance(target, TransferTarget.SPARK):
            destination = target.address
        elif isinstance(target, TransferTarget.LIGHTNING):
            destination = target.bolt11
        elif isinstance(target, TransferTarget.COOP_EXIT):
            destination = target.address
        logging.debug(
            f"Approve sending {unsigned.amount_sat} sats"
            f" (fee {unsigned.fee_sat} sats) to {destination}"
        )
        signature = TransferSignature.TRANSFER(
            signed=await signer.prepare_transfer(unsigned.prepare_transfer)
        )
    elif isinstance(unsigned, UnsignedTransferPackage.SWAP):
        logging.debug(
            f"Approve re-shaping funds for a {unsigned.amount_sat} sat send"
            f" (fee {unsigned.fee_sat} sats)"
        )
        signature = TransferSignature.TRANSFER(
            signed=await signer.prepare_transfer(unsigned.prepare_transfer)
        )
    elif isinstance(unsigned, UnsignedTransferPackage.TOKEN):
        logging.debug(
            f"Approve sending {unsigned.amount} of token"
            f" {unsigned.token_identifier} (fee {unsigned.fee})"
        )
        signature = TransferSignature.TOKEN(
            signed=await signer.prepare_token_transaction(
                unsigned.prepare_token_transaction
            )
        )
    else:
        raise ValueError("Unknown transfer package variant")

    signed_package = SignedTransferPackage(unsigned=unsigned, signature=signature)
    # ANCHOR_END: client-signing-sign-package
    return signed_package


async def send_with_client_signing(
    sdk: BreezSdk, signer: ExternalSparkSigner
) -> Payment:
    # ANCHOR: client-signing-send
    try:
        prepare_response = await sdk.prepare_send_payment(
            PrepareSendPaymentRequest(
                payment_request=PaymentRequest.INPUT(input="<spark address or invoice>"),
                amount=5_000,
                token_identifier=None,
                conversion_options=None,
                fee_policy=None,
            )
        )

        while True:
            unsigned = await sdk.build_unsigned_transfer_package(
                BuildUnsignedTransferPackageRequest(
                    prepare_response=prepare_response, options=None
                )
            )

            # Send the package to the user, who reviews and signs it
            signed_package = await sign_package(signer, unsigned)

            result = await sdk.publish_signed_transfer_package(
                PublishSignedTransferPackageRequest(signed_package=signed_package)
            )
            if isinstance(result, PublishSignedTransferPackageResponse.SWAP_COMPLETED):
                # The wallet's funds were re-shaped first: build the payment again
                continue
            if isinstance(result, PublishSignedTransferPackageResponse.PAYMENT_SENT):
                return result.payment
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: client-signing-send


async def build_onchain_package(
    sdk: BreezSdk, prepare_response: PrepareSendPaymentResponse
):
    # ANCHOR: client-signing-build-onchain-options
    # For Bitcoin address sends, the confirmation speed is chosen when
    # building the package: the fee depends on it
    try:
        unsigned = await sdk.build_unsigned_transfer_package(
            BuildUnsignedTransferPackageRequest(
                prepare_response=prepare_response,
                options=BuildTransferPackageOptions.BITCOIN_ADDRESS(
                    confirmation_speed=OnchainConfirmationSpeed.MEDIUM
                ),
            )
        )
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: client-signing-build-onchain-options
    logging.debug(f"Unsigned package: {unsigned}")


async def build_bolt11_package(
    sdk: BreezSdk, prepare_response: PrepareSendPaymentResponse
):
    # ANCHOR: client-signing-build-bolt11-options
    try:
        unsigned = await sdk.build_unsigned_transfer_package(
            BuildUnsignedTransferPackageRequest(
                prepare_response=prepare_response,
                options=BuildTransferPackageOptions.BOLT11_INVOICE(
                    prefer_spark=True, completion_timeout_secs=10
                ),
            )
        )
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: client-signing-build-bolt11-options
    logging.debug(f"Unsigned package: {unsigned}")


async def lnurl_pay_with_client_signing(
    sdk: BreezSdk,
    signer: ExternalSparkSigner,
    prepare_response: PrepareLnurlPayResponse,
) -> LnurlPayResponse:
    # ANCHOR: client-signing-lnurl-pay
    try:
        while True:
            unsigned = await sdk.build_unsigned_lnurl_pay_package(
                BuildUnsignedLnurlPayPackageRequest(prepare_response=prepare_response)
            )

            signed_package = await sign_package(signer, unsigned)

            result = await sdk.publish_signed_lnurl_pay_package(
                PublishSignedLnurlPayPackageRequest(signed_package=signed_package)
            )
            if isinstance(result, PublishSignedLnurlPayResponse.SWAP_COMPLETED):
                continue
            if isinstance(result, PublishSignedLnurlPayResponse.PAYMENT_SENT):
                return result.response
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: client-signing-lnurl-pay
