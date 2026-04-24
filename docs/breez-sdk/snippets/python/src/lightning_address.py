from breez_sdk_spark import (
    BreezSdk,
    CheckLightningAddressRequest,
    GetPaymentRequest,
    LightningAddressTransfer,
    Network,
    PaymentDetails,
    RegisterLightningAddressRequest,
    SignMessageRequest,
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
    lnurl_url = address_info.lnurl.url
    lnurl_bech32 = address_info.lnurl.bech32
    # ANCHOR_END: register-lightning-address
    return address_info


async def get_lightning_address(sdk: BreezSdk):
    # ANCHOR: get-lightning-address
    address_info_opt = await sdk.get_lightning_address()

    if address_info_opt is not None:
        lightning_address = address_info_opt.lightning_address
        username = address_info_opt.username
        description = address_info_opt.description
        lnurl_url = address_info_opt.lnurl.url
        lnurl_bech32 = address_info_opt.lnurl.bech32
    # ANCHOR_END: get-lightning-address


# Run on the *current owner's* wallet. Produces the authorization that the
# new owner needs to take over the username in a single atomic call.
async def sign_lightning_address_transfer(
    current_owner_sdk: BreezSdk,
    current_owner_pubkey: str,
    new_owner_pubkey: str,
) -> LightningAddressTransfer:
    username = "myusername"

    # ANCHOR: sign-lightning-address-transfer
    # `username` must be lowercased and trimmed.
    # pubkeys are hex-encoded secp256k1 compressed (via get_info().identity_pubkey).
    message = f"transfer:{current_owner_pubkey}-{username}-{new_owner_pubkey}"
    signed = await current_owner_sdk.sign_message(
        SignMessageRequest(message=message, compact=False)
    )

    transfer = LightningAddressTransfer(
        pubkey=signed.pubkey,
        signature=signed.signature,
    )
    # ANCHOR_END: sign-lightning-address-transfer
    return transfer


# Run on the *new owner's* wallet with the authorization received
# out-of-band from the current owner.
async def register_lightning_address_via_transfer(
    new_owner_sdk: BreezSdk,
    transfer: LightningAddressTransfer,
):
    username = "myusername"
    description = "My Lightning Address"

    # ANCHOR: register-lightning-address-transfer
    request = RegisterLightningAddressRequest(
        username=username,
        description=description,
        transfer=transfer,
    )

    address_info = await new_owner_sdk.register_lightning_address(request)
    # ANCHOR_END: register-lightning-address-transfer
    return address_info


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
