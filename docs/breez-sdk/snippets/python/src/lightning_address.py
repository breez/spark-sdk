from breez_sdk_spark import (
    BreezSdk,
    CheckLightningAddressRequest,
    GetPaymentRequest,
    Network,
    PaymentDetails,
    RegisterLightningAddressRequest,
    default_config
)

def configure_lightning_address():
    # ANCHOR: config-lightning-address
    config = default_config(network=Network.MAINNET)
    config.api_key = "your-api-key"
    config.lnurl_domain = "yourdomain.com"
    # ANCHOR_END: config-lightning-address
    return config

async def check_lightning_address_availability(sdk: BreezSdk, username: str) -> bool:
    username = "myusername"

    # ANCHOR: check-lightning-address
    request = CheckLightningAddressRequest(username=username)
    is_available = await sdk.check_lightning_address_available(request)
    # ANCHOR_END: check-lightning-address
    return is_available


async def register_lightning_address(sdk: BreezSdk, username: str, description: str):
    username = "myusername"
    description = "My Lightning Address"

    # ANCHOR: register-lightning-address
    request = RegisterLightningAddressRequest(
        username=username,
        description=description
    )

    address_info = await sdk.register_lightning_address(request)
    lightning_address = address_info.lightning_address
    lnurl = address_info.lnurl
    # ANCHOR_END: register-lightning-address
    return address_info


async def get_lightning_address(sdk: BreezSdk):
    # ANCHOR: get-lightning-address
    address_info_opt = await sdk.get_lightning_address()

    if address_info_opt is not None:
        lightning_address = address_info_opt.lightning_address
        username = address_info_opt.username
        description = address_info_opt.description
        lnurl = address_info_opt.lnurl
    # ANCHOR_END: get-lightning-address


async def delete_lightning_address(sdk: BreezSdk):
    # ANCHOR: delete-lightning-address
    await sdk.delete_lightning_address()
    # ANCHOR_END: delete-lightning-address


async def access_sender_comment(sdk: BreezSdk):
    payment_id = "<payment id>"
    response = await sdk.get_payment(GetPaymentRequest(payment_id=payment_id))
    payment = response.payment

    # ANCHOR: access-sender-comment
    # Check if this is a lightning payment with LNURL receive metadata
    if isinstance(payment.details, PaymentDetails.LIGHTNING):
        metadata = payment.details.lnurl_receive_metadata

        # Access the sender comment if present
        if metadata is not None and metadata.sender_comment is not None:
            print(f"Sender comment: {metadata.sender_comment}")
    # ANCHOR_END: access-sender-comment


async def access_nostr_zap(sdk: BreezSdk):
    payment_id = "<payment id>"
    response = await sdk.get_payment(GetPaymentRequest(payment_id=payment_id))
    payment = response.payment

    # ANCHOR: access-nostr-zap
    # Check if this is a lightning payment with LNURL receive metadata
    if isinstance(payment.details, PaymentDetails.LIGHTNING):
        metadata = payment.details.lnurl_receive_metadata

        if metadata is not None:
            # Access the Nostr zap request if present
            if metadata.nostr_zap_request is not None:
                # The nostr_zap_request is a JSON string containing the Nostr event (kind 9734)
                print(f"Nostr zap request: {metadata.nostr_zap_request}")

            # Access the Nostr zap receipt if present
            if metadata.nostr_zap_receipt is not None:
                # The nostr_zap_receipt is a JSON string containing the Nostr event (kind 9735)
                print(f"Nostr zap receipt: {metadata.nostr_zap_receipt}")
    # ANCHOR_END: access-nostr-zap
