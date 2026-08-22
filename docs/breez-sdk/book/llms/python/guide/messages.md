# Signing and verifying messages

Through signing and verifying messages we can provide proof that a digital signature was created by a private key.

## Signing a message

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.sign_message

By signing a message using the SDK we can provide a digital signature. Anyone with the `message`, `pubkey` and `signature` can verify the signature was created by the private key of this pubkey.

```python
message = "<message to sign>"
# Set to true to get a compact signature rather than a DER
compact = True
try:
    sign_message_request = SignMessageRequest(
        message=message, compact=compact
    )
    sign_message_response = await sdk.sign_message(request=sign_message_request)

    signature = sign_message_response.signature
    pubkey = sign_message_response.pubkey

    logging.debug(f"Pubkey: {pubkey}")
    logging.debug(f"Signature: {signature}")
except Exception as error:
    logging.error(error)
    raise
```



## Verifying a message

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.check_message

You can prove control of a private key by verifying a `message` with it's `signature` and `pubkey`.

```python
message = "<message>"
pubkey = "<pubkey of signer>"
signature = "<message signature>"
try:
    check_message_request = CheckMessageRequest(
        message=message, pubkey=pubkey, signature=signature
    )
    check_message_response = await sdk.check_message(request=check_message_request)

    is_valid = check_message_response.is_valid

    logging.debug(f"Signature valid: {is_valid}")
except Exception as error:
    logging.error(error)
    raise
```
