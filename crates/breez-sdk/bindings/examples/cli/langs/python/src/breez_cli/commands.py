import argparse
import hashlib
import math
import os
import time

import breez_sdk_spark
from breez_sdk_spark import (
    AssetFilter,
    BuyBitcoinRequest,
    CheckLightningAddressRequest,
    ClaimDepositRequest,
    ClaimHtlcPaymentRequest,
    ConversionOptions,
    ConversionType,
    Fee,
    FeePolicy,
    FetchConversionLimitsRequest,
    GetInfoRequest,
    GetPaymentRequest,
    GetTokensMetadataRequest,
    InputType,
    ListPaymentsRequest,
    ListUnclaimedDepositsRequest,
    LnurlPayRequest,
    LnurlWithdrawRequest,
    MaxFee,
    OnchainConfirmationSpeed,
    PaymentDetailsFilter,
    PaymentRequest,
    PaymentStatus,
    PaymentType,
    PrepareLnurlPayRequest,
    PrepareSendPaymentRequest,
    ReceivePaymentMethod,
    ReceivePaymentRequest,
    RefundDepositRequest,
    RegisterLightningAddressRequest,
    SendPaymentMethod,
    SendPaymentOptions,
    SendPaymentRequest,
    SparkHtlcOptions,
    SparkHtlcStatus,
    SyncWalletRequest,
    TokenTransactionType,
    UpdateUserSettingsRequest,
)

from breez_cli.serialization import print_value

# List of all top-level command names (used for REPL completion)
COMMAND_NAMES = [
    "get-info",
    "get-payment",
    "sync",
    "list-payments",
    "receive",
    "pay",
    "lnurl-pay",
    "lnurl-withdraw",
    "lnurl-auth",
    "claim-htlc-payment",
    "claim-deposit",
    "parse",
    "refund-deposit",
    "list-unclaimed-deposits",
    "buy-bitcoin",
    "check-lightning-address-available",
    "get-lightning-address",
    "register-lightning-address",
    "delete-lightning-address",
    "list-fiat-currencies",
    "list-fiat-rates",
    "recommended-fees",
    "get-tokens-metadata",
    "fetch-conversion-limits",
    "get-user-settings",
    "set-user-settings",
    "get-spark-status",
]


# ---------------------------------------------------------------------------
# Argument parser helpers
# ---------------------------------------------------------------------------

def _parser(name, description=""):
    """Create an ArgumentParser that doesn't call sys.exit on error."""
    p = argparse.ArgumentParser(prog=name, description=description)
    return p


def _add_bool_option(parser, *args, **kwargs):
    """Add a boolean option that accepts true/false values."""
    kwargs.setdefault("type", lambda v: v.lower() in ("true", "1", "yes"))
    kwargs.setdefault("default", None)
    parser.add_argument(*args, **kwargs)


# ---------------------------------------------------------------------------
# Parsers + handlers for each command
# ---------------------------------------------------------------------------

# --- get-info ---

def _build_get_info_parser():
    p = _parser("get-info", "Get balance information")
    _add_bool_option(p, "-e", "--ensure-synced")
    return p

async def _handle_get_info(sdk, _token_issuer, _session, args):
    result = await sdk.get_info(request=GetInfoRequest(ensure_synced=args.ensure_synced))
    print_value(result)


# --- get-payment ---

def _build_get_payment_parser():
    p = _parser("get-payment", "Get the payment with the given ID")
    p.add_argument("payment_id", help="The ID of the payment to retrieve")
    return p

async def _handle_get_payment(sdk, _token_issuer, _session, args):
    result = await sdk.get_payment(request=GetPaymentRequest(payment_id=args.payment_id))
    print_value(result)


# --- sync ---

def _build_sync_parser():
    return _parser("sync", "Sync wallet state")

async def _handle_sync(sdk, _token_issuer, _session, _args):
    result = await sdk.sync_wallet(request=SyncWalletRequest())
    print_value(result)


# --- list-payments ---

def _build_list_payments_parser():
    p = _parser("list-payments", "List payments")
    p.add_argument("-t", "--type-filter", nargs="*", default=None,
                   help="Filter by payment type (send, receive)")
    p.add_argument("-s", "--status-filter", nargs="*", default=None,
                   help="Filter by status (completed, pending, failed)")
    p.add_argument("-a", "--asset-filter", default=None,
                   help="Filter by asset (bitcoin, or a token identifier)")
    p.add_argument("--spark-htlc-status-filter", nargs="*", default=None,
                   help="Filter by Spark HTLC status")
    p.add_argument("--tx-hash", default=None, help="Filter by token transaction hash")
    p.add_argument("--tx-type", default=None, help="Filter by token transaction type")
    p.add_argument("--from-timestamp", type=int, default=None)
    p.add_argument("--to-timestamp", type=int, default=None)
    p.add_argument("-l", "--limit", type=int, default=10)
    p.add_argument("-o", "--offset", type=int, default=0)
    p.add_argument("--sort-ascending", type=lambda v: v.lower() in ("true", "1", "yes"), default=None)
    return p

def _parse_payment_type(s):
    mapping = {"send": PaymentType.SEND, "receive": PaymentType.RECEIVE}
    return mapping.get(s.lower())

def _parse_payment_status(s):
    mapping = {
        "completed": PaymentStatus.COMPLETED,
        "pending": PaymentStatus.PENDING,
        "failed": PaymentStatus.FAILED,
    }
    return mapping.get(s.lower())

def _parse_htlc_status(s):
    mapping = {
        "waiting_for_preimage": SparkHtlcStatus.WAITING_FOR_PREIMAGE,
        "preimage_received": SparkHtlcStatus.PREIMAGE_RECEIVED,
    }
    return mapping.get(s.lower())

def _parse_tx_type(s):
    mapping = {
        "mint": TokenTransactionType.MINT,
        "burn": TokenTransactionType.BURN,
        "transfer": TokenTransactionType.TRANSFER,
    }
    return mapping.get(s.lower())

async def _handle_list_payments(sdk, _token_issuer, _session, args):
    type_filter = None
    if args.type_filter:
        type_filter = [_parse_payment_type(t) for t in args.type_filter if _parse_payment_type(t)]

    status_filter = None
    if args.status_filter:
        status_filter = [_parse_payment_status(s) for s in args.status_filter if _parse_payment_status(s)]

    asset_filter = None
    if args.asset_filter:
        if args.asset_filter.lower() == "bitcoin":
            asset_filter = AssetFilter.BITCOIN
        else:
            asset_filter = AssetFilter.TOKEN(token_identifier=args.asset_filter)

    payment_details_filter = []
    if args.spark_htlc_status_filter:
        statuses = [_parse_htlc_status(s) for s in args.spark_htlc_status_filter if _parse_htlc_status(s)]
        if statuses:
            payment_details_filter.append(
                PaymentDetailsFilter.SPARK(htlc_status=statuses, conversion_refund_needed=None)
            )
    if args.tx_hash:
        payment_details_filter.append(
            PaymentDetailsFilter.TOKEN(
                conversion_refund_needed=None, tx_type=None, tx_hash=args.tx_hash,
            )
        )
    if args.tx_type:
        tx_type = _parse_tx_type(args.tx_type)
        if tx_type:
            payment_details_filter.append(
                PaymentDetailsFilter.TOKEN(
                    conversion_refund_needed=None, tx_type=tx_type, tx_hash=None,
                )
            )

    result = await sdk.list_payments(request=ListPaymentsRequest(
        limit=args.limit,
        offset=args.offset,
        type_filter=type_filter,
        status_filter=status_filter,
        asset_filter=asset_filter,
        payment_details_filter=payment_details_filter if payment_details_filter else None,
        from_timestamp=args.from_timestamp,
        to_timestamp=args.to_timestamp,
        sort_ascending=args.sort_ascending,
    ))
    print_value(result)


# --- receive ---

def _build_receive_parser():
    p = _parser("receive", "Receive a payment")
    p.add_argument("-m", "--method", required=True,
                   help="Payment method: sparkaddress, sparkinvoice, bitcoin, bolt11")
    p.add_argument("-d", "--description", default=None)
    p.add_argument("-a", "--amount", type=int, default=None)
    p.add_argument("-t", "--token-identifier", default=None)
    p.add_argument("-e", "--expiry-secs", type=int, default=None)
    p.add_argument("-s", "--sender-public-key", default=None)
    p.add_argument("--hodl", action="store_true", default=False,
                   help="Create a HODL invoice (bolt11 only)")
    p.add_argument("--new-address", action="store_true", default=False,
                   help="Request a new bitcoin deposit address instead of reusing the current one")
    return p

async def _handle_receive(sdk, _token_issuer, _session, args):
    method = args.method.lower()

    if method == "sparkaddress":
        payment_method = ReceivePaymentMethod.SPARK_ADDRESS()
    elif method == "sparkinvoice":
        expiry_time = None
        if args.expiry_secs is not None:
            expiry_time = int(time.time()) + args.expiry_secs
        payment_method = ReceivePaymentMethod.SPARK_INVOICE(
            amount=args.amount,
            token_identifier=args.token_identifier,
            expiry_time=expiry_time,
            description=args.description,
            sender_public_key=args.sender_public_key,
        )
    elif method == "bitcoin":
        payment_method = ReceivePaymentMethod.BITCOIN_ADDRESS(
            new_address=args.new_address if args.new_address else None,
        )
    elif method == "bolt11":
        payment_hash = None
        if args.hodl:
            preimage_bytes = os.urandom(32)
            preimage = preimage_bytes.hex()
            payment_hash = hashlib.sha256(preimage_bytes).hexdigest()
            print(f"HODL invoice preimage: {preimage}")
            print(f"Payment hash: {payment_hash}")
            print("Save the preimage! Use `claim-htlc-payment` with it to settle.")

        payment_method = ReceivePaymentMethod.BOLT11_INVOICE(
            description=args.description or "",
            amount_sats=args.amount,
            expiry_secs=args.expiry_secs,
            payment_hash=payment_hash,
        )
    else:
        print(f"Invalid payment method: {method}")
        return

    result = await sdk.receive_payment(request=ReceivePaymentRequest(payment_method=payment_method))

    if result.fee > 0:
        print(f"Prepared payment requires fee of {result.fee} sats/token base units\n")

    print_value(result)


# --- pay ---

def _build_pay_parser():
    p = _parser("pay", "Pay the given payment request")
    p.add_argument("-r", "--payment-request", required=True)
    p.add_argument("-a", "--amount", type=int, default=None)
    p.add_argument("-t", "--token-identifier", default=None)
    p.add_argument("-i", "--idempotency-key", default=None)
    p.add_argument("--from-bitcoin", action="store_true", default=False)
    p.add_argument("--from-token", default=None)
    p.add_argument("-s", "--convert-max-slippage-bps", type=int, default=None)
    p.add_argument("--fees-included", action="store_true", default=False)
    return p

async def _handle_pay(sdk, _token_issuer, session, args):
    conversion_options = None
    if args.from_bitcoin:
        conversion_options = ConversionOptions(
            conversion_type=ConversionType.FROM_BITCOIN(),
            max_slippage_bps=args.convert_max_slippage_bps,
            completion_timeout_secs=None,
        )
    elif args.from_token:
        conversion_options = ConversionOptions(
            conversion_type=ConversionType.TO_BITCOIN(from_token_identifier=args.from_token),
            max_slippage_bps=args.convert_max_slippage_bps,
            completion_timeout_secs=None,
        )

    fee_policy = FeePolicy.FEES_INCLUDED if args.fees_included else None

    prepare_response = await sdk.prepare_send_payment(
        request=PrepareSendPaymentRequest(
            payment_request=PaymentRequest.INPUT(args.payment_request),
            amount=args.amount,
            token_identifier=args.token_identifier,
            conversion_options=conversion_options,
            fee_policy=fee_policy,
        )
    )

    if prepare_response.conversion_estimate is not None:
        est = prepare_response.conversion_estimate
        units = "sats" if isinstance(est.options.conversion_type, ConversionType.FROM_BITCOIN) else "token base units"
        print(f"Estimated conversion of {est.amount} {units} with a {est.fee} {units} fee")
        line = await session.prompt_async("Do you want to continue (y/n): ", default="y")
        if line.strip().lower() != "y":
            print("Payment cancelled")
            return

    payment_options = await read_payment_options(prepare_response.payment_method, session)

    send_response = await sdk.send_payment(
        request=SendPaymentRequest(
            prepare_response=prepare_response,
            options=payment_options,
            idempotency_key=args.idempotency_key,
        )
    )
    print_value(send_response)


# --- lnurl-pay ---

def _build_lnurl_pay_parser():
    p = _parser("lnurl-pay", "Pay using LNURL")
    p.add_argument("lnurl", help="LN Address or LNURL-pay endpoint")
    p.add_argument("-c", "--comment", default=None)
    p.add_argument("-v", "--validate", type=lambda v: v.lower() in ("true", "1", "yes"), default=None,
                   dest="validate_success_url")
    p.add_argument("-i", "--idempotency-key", default=None)
    p.add_argument("--from-token", default=None)
    p.add_argument("-s", "--convert-max-slippage-bps", type=int, default=None)
    p.add_argument("--fees-included", action="store_true", default=False)
    return p

async def _handle_lnurl_pay(sdk, _token_issuer, session, args):
    conversion_options = None
    if args.from_token:
        conversion_options = ConversionOptions(
            conversion_type=ConversionType.TO_BITCOIN(from_token_identifier=args.from_token),
            max_slippage_bps=args.convert_max_slippage_bps,
            completion_timeout_secs=None,
        )

    fee_policy = FeePolicy.FEES_INCLUDED if args.fees_included else None

    parsed = await sdk.parse(input=args.lnurl)

    pay_request = None
    if isinstance(parsed, InputType.LIGHTNING_ADDRESS):
        pay_request = parsed[0].pay_request
    elif isinstance(parsed, InputType.LNURL_PAY):
        pay_request = parsed[0]
    else:
        print("Invalid input: expected LNURL-pay or Lightning address")
        return

    min_sendable = math.ceil(pay_request.min_sendable / 1000)
    max_sendable = pay_request.max_sendable // 1000
    prompt = f"Amount to pay (min {min_sendable} sat, max {max_sendable} sat): "
    amount_str = await session.prompt_async(prompt)
    amount_sats = int(amount_str.strip())

    prepare_response = await sdk.prepare_lnurl_pay(
        request=PrepareLnurlPayRequest(
            amount_sats=amount_sats,
            comment=args.comment,
            pay_request=pay_request,
            validate_success_action_url=args.validate_success_url,
            conversion_options=conversion_options,
            fee_policy=fee_policy,
        )
    )

    if prepare_response.conversion_estimate is not None:
        est = prepare_response.conversion_estimate
        print(f"Estimated conversion of {est.amount} token base units with a {est.fee} token base units fee")
        line = await session.prompt_async("Do you want to continue (y/n): ", default="y")
        if line.strip().lower() != "y":
            print("Payment cancelled")
            return

    print_value(prepare_response)
    line = await session.prompt_async("Do you want to continue? (y/n): ", default="y")
    if line.strip().lower() != "y":
        return

    result = await sdk.lnurl_pay(
        request=LnurlPayRequest(
            prepare_response=prepare_response,
            idempotency_key=args.idempotency_key,
        )
    )
    print_value(result)


# --- lnurl-withdraw ---

def _build_lnurl_withdraw_parser():
    p = _parser("lnurl-withdraw", "Withdraw using LNURL")
    p.add_argument("lnurl", help="LNURL-withdraw endpoint")
    p.add_argument("-t", "--timeout", type=int, default=None, dest="completion_timeout_secs")
    return p

async def _handle_lnurl_withdraw(sdk, _token_issuer, session, args):
    parsed = await sdk.parse(input=args.lnurl)

    if not isinstance(parsed, InputType.LNURL_WITHDRAW):
        print("Invalid input: expected LNURL-withdraw")
        return

    withdraw_request = parsed[0]
    min_withdrawable = math.ceil(withdraw_request.min_withdrawable / 1000)
    max_withdrawable = withdraw_request.max_withdrawable // 1000
    prompt = f"Amount to withdraw (min {min_withdrawable} sat, max {max_withdrawable} sat): "
    amount_str = await session.prompt_async(prompt)
    amount_sats = int(amount_str.strip())

    result = await sdk.lnurl_withdraw(
        request=LnurlWithdrawRequest(
            amount_sats=amount_sats,
            withdraw_request=withdraw_request,
            completion_timeout_secs=args.completion_timeout_secs,
        )
    )
    print_value(result)


# --- lnurl-auth ---

def _build_lnurl_auth_parser():
    p = _parser("lnurl-auth", "Authenticate using LNURL")
    p.add_argument("lnurl", help="LNURL-auth endpoint")
    return p

async def _handle_lnurl_auth(sdk, _token_issuer, session, args):
    parsed = await sdk.parse(input=args.lnurl)

    if not isinstance(parsed, InputType.LNURL_AUTH):
        print("Invalid input: expected LNURL-auth")
        return

    auth_request = parsed[0]
    action = auth_request.action or "auth"
    prompt = f"Authenticate with {auth_request.domain} (action: {action})? (y/n): "
    line = await session.prompt_async(prompt, default="y")
    if line.strip().lower() != "y":
        return

    result = await sdk.lnurl_auth(request_data=auth_request)
    print_value(result)


# --- claim-htlc-payment ---

def _build_claim_htlc_payment_parser():
    p = _parser("claim-htlc-payment", "Claim an HTLC payment")
    p.add_argument("preimage", help="The preimage of the HTLC (hex string)")
    return p

async def _handle_claim_htlc_payment(sdk, _token_issuer, _session, args):
    result = await sdk.claim_htlc_payment(request=ClaimHtlcPaymentRequest(preimage=args.preimage))
    print_value(result.payment)


# --- claim-deposit ---

def _build_claim_deposit_parser():
    p = _parser("claim-deposit", "Claim an on-chain deposit")
    p.add_argument("txid", help="The txid of the deposit")
    p.add_argument("vout", type=int, help="The vout of the deposit")
    p.add_argument("--fee-sat", type=int, default=None, help="Max fee in sats")
    p.add_argument("--sat-per-vbyte", type=int, default=None, help="Max fee per vbyte")
    p.add_argument("--recommended-fee-leeway", type=int, default=None,
                   help="Use recommended fee + leeway")
    return p

async def _handle_claim_deposit(sdk, _token_issuer, _session, args):
    if args.recommended_fee_leeway is not None:
        if args.fee_sat is not None or args.sat_per_vbyte is not None:
            print("Cannot specify fee_sat or sat_per_vbyte when using recommended fee")
            return
        max_fee = MaxFee.NETWORK_RECOMMENDED(leeway_sat_per_vbyte=args.recommended_fee_leeway)
    elif args.fee_sat is not None and args.sat_per_vbyte is not None:
        print("Cannot specify both fee_sat and sat_per_vbyte")
        return
    elif args.fee_sat is not None:
        max_fee = MaxFee.FIXED(amount=args.fee_sat)
    elif args.sat_per_vbyte is not None:
        max_fee = MaxFee.RATE(sat_per_vbyte=args.sat_per_vbyte)
    else:
        max_fee = None

    result = await sdk.claim_deposit(request=ClaimDepositRequest(
        txid=args.txid,
        vout=args.vout,
        max_fee=max_fee,
    ))
    print_value(result)


# --- parse ---

def _build_parse_parser():
    p = _parser("parse", "Parse an input (invoice, address, LNURL)")
    p.add_argument("input", help="The input to parse")
    return p

async def _handle_parse(sdk, _token_issuer, _session, args):
    result = await sdk.parse(input=args.input)
    print_value(result)


# --- refund-deposit ---

def _build_refund_deposit_parser():
    p = _parser("refund-deposit", "Refund an on-chain deposit")
    p.add_argument("txid", help="The txid of the deposit")
    p.add_argument("vout", type=int, help="The vout of the deposit")
    p.add_argument("destination_address", help="Destination address")
    p.add_argument("--fee-sat", type=int, default=None)
    p.add_argument("--sat-per-vbyte", type=int, default=None)
    return p

async def _handle_refund_deposit(sdk, _token_issuer, _session, args):
    if args.fee_sat is not None and args.sat_per_vbyte is not None:
        print("Cannot specify both fee_sat and sat_per_vbyte")
        return
    if args.fee_sat is not None:
        fee = Fee.FIXED(amount=args.fee_sat)
    elif args.sat_per_vbyte is not None:
        fee = Fee.RATE(sat_per_vbyte=args.sat_per_vbyte)
    else:
        print("Must specify either --fee-sat or --sat-per-vbyte")
        return

    result = await sdk.refund_deposit(request=RefundDepositRequest(
        txid=args.txid,
        vout=args.vout,
        destination_address=args.destination_address,
        fee=fee,
    ))
    print_value(result)


# --- list-unclaimed-deposits ---

def _build_list_unclaimed_deposits_parser():
    return _parser("list-unclaimed-deposits", "List unclaimed on-chain deposits")

async def _handle_list_unclaimed_deposits(sdk, _token_issuer, _session, _args):
    result = await sdk.list_unclaimed_deposits(request=ListUnclaimedDepositsRequest())
    print_value(result)


# --- buy-bitcoin ---

def _build_buy_bitcoin_parser():
    p = _parser("buy-bitcoin", "Buy Bitcoin using an external provider")
    p.add_argument("--provider", default="moonpay",
                   help='Provider to use: "moonpay" (default) or "cashapp"')
    p.add_argument("--amount-sat", type=int, default=None,
                   help="Amount in satoshis (meaning depends on provider)")
    p.add_argument("--redirect-url", default=None,
                   help="Custom redirect URL after purchase completion (MoonPay only)")
    return p

async def _handle_buy_bitcoin(sdk, _token_issuer, _session, args):
    provider = (args.provider or "moonpay").lower()
    if provider in ("cashapp", "cash_app", "cash-app"):
        request = BuyBitcoinRequest.CASH_APP(amount_sats=args.amount_sat)
    else:
        request = BuyBitcoinRequest.MOONPAY(
            locked_amount_sat=args.amount_sat,
            redirect_url=args.redirect_url,
        )
    result = await sdk.buy_bitcoin(request=request)
    print("Open this URL in a browser to complete the purchase:")
    print(result.url)


# --- check-lightning-address-available ---

def _build_check_lightning_address_available_parser():
    p = _parser("check-lightning-address-available", "Check if a lightning address username is available")
    p.add_argument("username", help="The username to check")
    return p

async def _handle_check_lightning_address_available(sdk, _token_issuer, _session, args):
    result = await sdk.check_lightning_address_available(
        request=CheckLightningAddressRequest(username=args.username)
    )
    print_value(result)


# --- get-lightning-address ---

def _build_get_lightning_address_parser():
    return _parser("get-lightning-address", "Get registered lightning address")

async def _handle_get_lightning_address(sdk, _token_issuer, _session, _args):
    result = await sdk.get_lightning_address()
    print_value(result)


# --- register-lightning-address ---

def _build_register_lightning_address_parser():
    p = _parser("register-lightning-address", "Register a lightning address")
    p.add_argument("username", help="The lightning address username")
    p.add_argument("description", nargs="?", default=None, help="Description for the lnurl response")
    return p

async def _handle_register_lightning_address(sdk, _token_issuer, _session, args):
    result = await sdk.register_lightning_address(
        request=RegisterLightningAddressRequest(
            username=args.username,
            description=args.description,
        )
    )
    print_value(result)


# --- delete-lightning-address ---

def _build_delete_lightning_address_parser():
    return _parser("delete-lightning-address", "Delete lightning address")

async def _handle_delete_lightning_address(sdk, _token_issuer, _session, _args):
    await sdk.delete_lightning_address()
    print("Lightning address deleted")


# --- list-fiat-currencies ---

def _build_list_fiat_currencies_parser():
    return _parser("list-fiat-currencies", "List fiat currencies")

async def _handle_list_fiat_currencies(sdk, _token_issuer, _session, _args):
    result = await sdk.list_fiat_currencies()
    print_value(result)


# --- list-fiat-rates ---

def _build_list_fiat_rates_parser():
    return _parser("list-fiat-rates", "List available fiat rates")

async def _handle_list_fiat_rates(sdk, _token_issuer, _session, _args):
    result = await sdk.list_fiat_rates()
    print_value(result)


# --- recommended-fees ---

def _build_recommended_fees_parser():
    return _parser("recommended-fees", "Get recommended BTC fees")

async def _handle_recommended_fees(sdk, _token_issuer, _session, _args):
    result = await sdk.recommended_fees()
    print_value(result)


# --- get-tokens-metadata ---

def _build_get_tokens_metadata_parser():
    p = _parser("get-tokens-metadata", "Get metadata for token(s)")
    p.add_argument("token_identifiers", nargs="+", help="Token identifiers")
    return p

async def _handle_get_tokens_metadata(sdk, _token_issuer, _session, args):
    result = await sdk.get_tokens_metadata(
        request=GetTokensMetadataRequest(token_identifiers=args.token_identifiers)
    )
    print_value(result)


# --- fetch-conversion-limits ---

def _build_fetch_conversion_limits_parser():
    p = _parser("fetch-conversion-limits", "Fetch conversion limits for a token")
    p.add_argument("-f", "--from-bitcoin", action="store_true", default=False)
    p.add_argument("token_identifier", help="The token identifier")
    return p

async def _handle_fetch_conversion_limits(sdk, _token_issuer, _session, args):
    if args.from_bitcoin:
        request = FetchConversionLimitsRequest(
            conversion_type=ConversionType.FROM_BITCOIN(),
            token_identifier=args.token_identifier,
        )
    else:
        request = FetchConversionLimitsRequest(
            conversion_type=ConversionType.TO_BITCOIN(from_token_identifier=args.token_identifier),
            token_identifier=None,
        )
    result = await sdk.fetch_conversion_limits(request=request)
    print_value(result)


# --- get-user-settings ---

def _build_get_user_settings_parser():
    return _parser("get-user-settings", "Get user settings")

async def _handle_get_user_settings(sdk, _token_issuer, _session, _args):
    result = await sdk.get_user_settings()
    print_value(result)


# --- set-user-settings ---

def _build_set_user_settings_parser():
    p = _parser("set-user-settings", "Update user settings")
    _add_bool_option(p, "-p", "--private", dest="spark_private_mode_enabled",
                     help="Whether spark private mode is enabled")
    return p

async def _handle_set_user_settings(sdk, _token_issuer, _session, args):
    await sdk.update_user_settings(
        request=UpdateUserSettingsRequest(
            spark_private_mode_enabled=args.spark_private_mode_enabled,
        )
    )
    print("User settings updated")


# --- get-spark-status ---

def _build_get_spark_status_parser():
    return _parser("get-spark-status", "Get Spark network service status")

async def _handle_get_spark_status(_sdk, _token_issuer, _session, _args):
    result = await breez_sdk_spark.get_spark_status()
    print_value(result)


# ---------------------------------------------------------------------------
# read_payment_options — interactive fee/option selection
# ---------------------------------------------------------------------------

async def read_payment_options(payment_method, session):
    """Prompt user for payment options based on the payment method type."""
    if isinstance(payment_method, SendPaymentMethod.BITCOIN_ADDRESS):
        fee_quote = payment_method.fee_quote
        fast_fee = fee_quote.speed_fast.user_fee_sat + fee_quote.speed_fast.l1_broadcast_fee_sat
        medium_fee = fee_quote.speed_medium.user_fee_sat + fee_quote.speed_medium.l1_broadcast_fee_sat
        slow_fee = fee_quote.speed_slow.user_fee_sat + fee_quote.speed_slow.l1_broadcast_fee_sat
        print("Please choose payment fee:")
        print(f"1. Fast: {fast_fee}")
        print(f"2. Medium: {medium_fee}")
        print(f"3. Slow: {slow_fee}")
        line = await session.prompt_async("", default="1")
        speed_map = {
            "1": OnchainConfirmationSpeed.FAST,
            "2": OnchainConfirmationSpeed.MEDIUM,
            "3": OnchainConfirmationSpeed.SLOW,
        }
        speed = speed_map.get(line.strip())
        if speed is None:
            raise ValueError("Invalid confirmation speed")
        return SendPaymentOptions.BITCOIN_ADDRESS(confirmation_speed=speed)

    if isinstance(payment_method, SendPaymentMethod.BOLT11_INVOICE):
        if payment_method.spark_transfer_fee_sats is not None:
            print("Choose payment option:")
            print(f"1. Spark transfer fee: {payment_method.spark_transfer_fee_sats} sats")
            print(f"2. Lightning fee: {payment_method.lightning_fee_sats} sats")
            line = await session.prompt_async("", default="1")
            if line.strip() == "1":
                return SendPaymentOptions.BOLT11_INVOICE(
                    prefer_spark=True, completion_timeout_secs=0,
                )
        return SendPaymentOptions.BOLT11_INVOICE(
            prefer_spark=False, completion_timeout_secs=0,
        )

    if isinstance(payment_method, SendPaymentMethod.SPARK_ADDRESS):
        # HTLC options are only valid for Bitcoin payments, not token payments
        if payment_method.token_identifier is not None:
            return None

        line = await session.prompt_async(
            "Do you want to create an HTLC transfer? (y/n)", default="n"
        )
        if line.strip().lower() != "y":
            return None

        payment_hash = await session.prompt_async(
            "Please enter the HTLC payment hash (hex string) or leave empty to generate: "
        )
        payment_hash = payment_hash.strip()
        if not payment_hash:
            preimage_bytes = os.urandom(32)
            preimage = preimage_bytes.hex()
            payment_hash = hashlib.sha256(preimage_bytes).hexdigest()
            print(f"Generated preimage: {preimage}")
            print(f"Associated payment hash: {payment_hash}")

        expiry_str = await session.prompt_async(
            "Please enter the HTLC expiry duration in seconds: "
        )
        expiry_duration_secs = int(expiry_str.strip())

        return SendPaymentOptions.SPARK_ADDRESS(
            htlc_options=SparkHtlcOptions(
                payment_hash=payment_hash,
                expiry_duration_secs=expiry_duration_secs,
            )
        )

    # SendPaymentMethod.SPARK_INVOICE
    return None


# ---------------------------------------------------------------------------
# Command registry
# ---------------------------------------------------------------------------

def build_command_registry():
    """Build and return the command registry mapping names to (parser, handler) tuples."""
    return {
        "get-info": (_build_get_info_parser(), _handle_get_info),
        "get-payment": (_build_get_payment_parser(), _handle_get_payment),
        "sync": (_build_sync_parser(), _handle_sync),
        "list-payments": (_build_list_payments_parser(), _handle_list_payments),
        "receive": (_build_receive_parser(), _handle_receive),
        "pay": (_build_pay_parser(), _handle_pay),
        "lnurl-pay": (_build_lnurl_pay_parser(), _handle_lnurl_pay),
        "lnurl-withdraw": (_build_lnurl_withdraw_parser(), _handle_lnurl_withdraw),
        "lnurl-auth": (_build_lnurl_auth_parser(), _handle_lnurl_auth),
        "claim-htlc-payment": (_build_claim_htlc_payment_parser(), _handle_claim_htlc_payment),
        "claim-deposit": (_build_claim_deposit_parser(), _handle_claim_deposit),
        "parse": (_build_parse_parser(), _handle_parse),
        "refund-deposit": (_build_refund_deposit_parser(), _handle_refund_deposit),
        "list-unclaimed-deposits": (_build_list_unclaimed_deposits_parser(), _handle_list_unclaimed_deposits),
        "buy-bitcoin": (_build_buy_bitcoin_parser(), _handle_buy_bitcoin),
        "check-lightning-address-available": (_build_check_lightning_address_available_parser(), _handle_check_lightning_address_available),
        "get-lightning-address": (_build_get_lightning_address_parser(), _handle_get_lightning_address),
        "register-lightning-address": (_build_register_lightning_address_parser(), _handle_register_lightning_address),
        "delete-lightning-address": (_build_delete_lightning_address_parser(), _handle_delete_lightning_address),
        "list-fiat-currencies": (_build_list_fiat_currencies_parser(), _handle_list_fiat_currencies),
        "list-fiat-rates": (_build_list_fiat_rates_parser(), _handle_list_fiat_rates),
        "recommended-fees": (_build_recommended_fees_parser(), _handle_recommended_fees),
        "get-tokens-metadata": (_build_get_tokens_metadata_parser(), _handle_get_tokens_metadata),
        "fetch-conversion-limits": (_build_fetch_conversion_limits_parser(), _handle_fetch_conversion_limits),
        "get-user-settings": (_build_get_user_settings_parser(), _handle_get_user_settings),
        "set-user-settings": (_build_set_user_settings_parser(), _handle_set_user_settings),
        "get-spark-status": (_build_get_spark_status_parser(), _handle_get_spark_status),
    }
