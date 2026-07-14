//! Grammar tests for the REPL command surface. They pin the exact clap
//! grammar (names, aliases, defaults, conflicts) that the language CLI ports
//! mirror, so a grammar change here is a deliberate, reviewed act.

use breez_sdk_spark::{
    AssetFilter, PaymentStatus, PaymentType, SparkHtlcStatus, TokenTransactionType,
};
use clap::Parser;

use super::contacts::ContactCommand;
use super::issuer::IssuerCommand;
use super::stable_balance::StableBalanceCommand;
use super::webhooks::{WebhookCommand, WebhookEventTypeArg};
use super::{Command, ReceivePaymentMethodArg};

fn parse(line: &str) -> Result<Command, clap::Error> {
    let mut args = vec!["breez-cli".to_string()];
    args.extend(shlex::split(line).expect("test input must be shlex-splittable"));
    Command::try_parse_from(args)
}

fn parse_ok(line: &str) -> Command {
    parse(line).unwrap_or_else(|e| panic!("'{line}' failed to parse: {e}"))
}

fn parse_err(line: &str) -> String {
    match parse(line) {
        Ok(_) => panic!("'{line}' parsed but should fail"),
        Err(e) => e.to_string(),
    }
}

#[test]
fn get_info() {
    assert!(matches!(
        parse_ok("get-info"),
        Command::GetInfo {
            ensure_synced: None
        }
    ));
    assert!(matches!(
        parse_ok("get-info -e true"),
        Command::GetInfo {
            ensure_synced: Some(true)
        }
    ));
    assert!(matches!(
        parse_ok("get-info --ensure-synced false"),
        Command::GetInfo {
            ensure_synced: Some(false)
        }
    ));
}

#[test]
fn get_payment() {
    let Command::GetPayment { payment_id } = parse_ok("get-payment abc123") else {
        panic!("expected GetPayment");
    };
    assert_eq!(payment_id, "abc123");
    parse_err("get-payment");
}

#[test]
fn sync() {
    assert!(matches!(parse_ok("sync"), Command::Sync));
}

#[test]
fn list_payments_defaults() {
    let Command::ListPayments {
        type_filter,
        status_filter,
        asset_filter,
        spark_htlc_status_filter,
        tx_hash,
        tx_type,
        from_timestamp,
        to_timestamp,
        limit,
        offset,
        sort_ascending,
    } = parse_ok("list-payments")
    else {
        panic!("expected ListPayments");
    };
    assert!(type_filter.is_none());
    assert!(status_filter.is_none());
    assert!(asset_filter.is_none());
    assert!(spark_htlc_status_filter.is_none());
    assert!(tx_hash.is_none());
    assert!(tx_type.is_none());
    assert!(from_timestamp.is_none());
    assert!(to_timestamp.is_none());
    assert_eq!(limit, Some(10));
    assert_eq!(offset, Some(0));
    assert!(sort_ascending.is_none());
}

#[test]
fn list_payments_filters() {
    let Command::ListPayments {
        type_filter,
        status_filter,
        asset_filter,
        spark_htlc_status_filter,
        tx_type,
        limit,
        offset,
        sort_ascending,
        ..
    } = parse_ok(
        "list-payments -t send -t receive -s completed -a token:tok1 \
         --spark-htlc-status-filter PreimageShared --tx-type mint \
         --limit 5 --offset 2 --sort-ascending true",
    )
    else {
        panic!("expected ListPayments");
    };
    assert!(matches!(
        type_filter.as_deref(),
        Some([PaymentType::Send, PaymentType::Receive])
    ));
    assert!(matches!(
        status_filter.as_deref(),
        Some([PaymentStatus::Completed])
    ));
    assert!(matches!(
        asset_filter,
        Some(AssetFilter::Token {
            token_identifier: Some(t)
        }) if t == "tok1"
    ));
    assert!(matches!(
        spark_htlc_status_filter.as_deref(),
        Some([SparkHtlcStatus::PreimageShared])
    ));
    assert!(matches!(tx_type, Some(TokenTransactionType::Mint)));
    assert_eq!(limit, Some(5));
    assert_eq!(offset, Some(2));
    assert_eq!(sort_ascending, Some(true));

    parse_err("list-payments -t onchain");
    parse_err("list-payments --limit ten");
}

#[test]
fn receive_methods() {
    for (line, expected) in [
        ("receive -m sparkaddress", "sparkaddress"),
        ("receive -m sparkinvoice", "sparkinvoice"),
        ("receive -m bitcoin", "bitcoin"),
        ("receive -m bolt11", "bolt11"),
        ("receive --method bolt11", "bolt11"),
    ] {
        let Command::Receive { payment_method, .. } = parse_ok(line) else {
            panic!("expected Receive for '{line}'");
        };
        let matches_expected = match payment_method {
            ReceivePaymentMethodArg::SparkAddress => expected == "sparkaddress",
            ReceivePaymentMethodArg::SparkInvoice => expected == "sparkinvoice",
            ReceivePaymentMethodArg::Bitcoin => expected == "bitcoin",
            ReceivePaymentMethodArg::Bolt11 => expected == "bolt11",
        };
        assert!(matches_expected, "'{line}' parsed the wrong method");
    }
    parse_err("receive");
    parse_err("receive -m lightning");
}

#[test]
fn receive_args() {
    let Command::Receive {
        description,
        amount,
        token_identifier,
        expiry_secs,
        sender_public_key,
        hodl,
        new_address,
        ..
    } = parse_ok(
        "receive -m bolt11 -d \"coffee and cake\" -a 2500 -t tok1 -e 3600 -s 02aa --hodl \
         --new-address",
    )
    else {
        panic!("expected Receive");
    };
    assert_eq!(description.as_deref(), Some("coffee and cake"));
    assert_eq!(amount, Some(2500));
    assert_eq!(token_identifier.as_deref(), Some("tok1"));
    assert_eq!(expiry_secs, Some(3600));
    assert_eq!(sender_public_key.as_deref(), Some("02aa"));
    assert!(hodl);
    assert!(new_address);

    let Command::Receive {
        hodl, new_address, ..
    } = parse_ok("receive -m bolt11")
    else {
        panic!("expected Receive");
    };
    assert!(!hodl);
    assert!(!new_address);
}

#[test]
fn pay() {
    let Command::Pay {
        payment_request,
        amount,
        token_identifier,
        idempotency_key,
        convert_from_bitcoin,
        convert_from_token_identifier,
        convert_max_slippage_bps,
        cross_chain_max_slippage_bps,
        fees_included,
    } = parse_ok(
        "pay -r lnbc1... -a 1000 -t tok1 -i key1 -s 40 --cross-chain-max-slippage-bps 100",
    )
    else {
        panic!("expected Pay");
    };
    assert_eq!(payment_request, "lnbc1...");
    assert_eq!(amount, Some(1000));
    assert_eq!(token_identifier.as_deref(), Some("tok1"));
    assert_eq!(idempotency_key.as_deref(), Some("key1"));
    assert_eq!(convert_from_bitcoin, Some(false));
    assert!(convert_from_token_identifier.is_none());
    assert_eq!(convert_max_slippage_bps, Some(40));
    assert_eq!(cross_chain_max_slippage_bps, Some(100));
    assert!(!fees_included);

    let Command::Pay {
        convert_from_bitcoin,
        fees_included,
        ..
    } = parse_ok("pay -r addr1 --from-bitcoin --fees-included")
    else {
        panic!("expected Pay");
    };
    assert_eq!(convert_from_bitcoin, Some(true));
    assert!(fees_included);

    let Command::Pay {
        convert_from_token_identifier,
        ..
    } = parse_ok("pay -r addr1 --from-token tok1")
    else {
        panic!("expected Pay");
    };
    assert_eq!(convert_from_token_identifier.as_deref(), Some("tok1"));

    parse_err("pay");
    let err = parse_err("pay -r addr1 --from-bitcoin --from-token tok1");
    assert!(
        err.contains("cannot be used with"),
        "unexpected error: {err}"
    );
}

#[test]
fn lnurl_pay() {
    let Command::LnurlPay {
        lnurl,
        comment,
        validate_success_url,
        idempotency_key,
        token_identifier,
        convert_from_token_identifier,
        convert_max_slippage_bps,
        fees_included,
    } = parse_ok(
        "lnurl-pay user@domain.com -c hello -v true -i key1 -t tok1 --from-token tok2 -s 30",
    )
    else {
        panic!("expected LnurlPay");
    };
    assert_eq!(lnurl, "user@domain.com");
    assert_eq!(comment.as_deref(), Some("hello"));
    assert_eq!(validate_success_url, Some(true));
    assert_eq!(idempotency_key.as_deref(), Some("key1"));
    assert_eq!(token_identifier.as_deref(), Some("tok1"));
    assert_eq!(convert_from_token_identifier.as_deref(), Some("tok2"));
    assert_eq!(convert_max_slippage_bps, Some(30));
    assert!(!fees_included);

    parse_err("lnurl-pay");
}

#[test]
fn lnurl_withdraw() {
    let Command::LnurlWithdraw {
        lnurl,
        completion_timeout_secs,
    } = parse_ok("lnurl-withdraw lnurl1... -t 30")
    else {
        panic!("expected LnurlWithdraw");
    };
    assert_eq!(lnurl, "lnurl1...");
    assert_eq!(completion_timeout_secs, Some(30));
}

#[test]
fn lnurl_auth() {
    let Command::LnurlAuth { lnurl } = parse_ok("lnurl-auth lnurl1...") else {
        panic!("expected LnurlAuth");
    };
    assert_eq!(lnurl, "lnurl1...");
}

#[test]
fn claim_htlc_payment() {
    let Command::ClaimHtlcPayment { preimage } = parse_ok("claim-htlc-payment deadbeef") else {
        panic!("expected ClaimHtlcPayment");
    };
    assert_eq!(preimage, "deadbeef");
}

#[test]
fn claim_deposit() {
    let Command::ClaimDeposit {
        txid,
        vout,
        fee_sat,
        sat_per_vbyte,
        recommended_fee_leeway,
    } = parse_ok("claim-deposit tx1 0 --fee-sat 500")
    else {
        panic!("expected ClaimDeposit");
    };
    assert_eq!(txid, "tx1");
    assert_eq!(vout, 0);
    assert_eq!(fee_sat, Some(500));
    assert!(sat_per_vbyte.is_none());
    assert!(recommended_fee_leeway.is_none());

    let Command::ClaimDeposit {
        sat_per_vbyte,
        recommended_fee_leeway,
        ..
    } = parse_ok("claim-deposit tx1 1 --sat-per-vbyte 2 --recommended-fee-leeway 3")
    else {
        panic!("expected ClaimDeposit");
    };
    assert_eq!(sat_per_vbyte, Some(2));
    assert_eq!(recommended_fee_leeway, Some(3));

    parse_err("claim-deposit tx1");
    parse_err("claim-deposit tx1 notanumber");
}

#[test]
fn parse_input() {
    let Command::Parse { input } = parse_ok("parse lnbc1...") else {
        panic!("expected Parse");
    };
    assert_eq!(input, "lnbc1...");
    parse_err("parse");
}

#[test]
fn refund_deposit() {
    let Command::RefundDeposit {
        txid,
        vout,
        destination_address,
        fee_sat,
        sat_per_vbyte,
    } = parse_ok("refund-deposit tx1 0 bcrt1qaddr --sat-per-vbyte 5")
    else {
        panic!("expected RefundDeposit");
    };
    assert_eq!(txid, "tx1");
    assert_eq!(vout, 0);
    assert_eq!(destination_address, "bcrt1qaddr");
    assert!(fee_sat.is_none());
    assert_eq!(sat_per_vbyte, Some(5));

    parse_err("refund-deposit tx1 0");
}

#[test]
fn list_unclaimed_deposits() {
    assert!(matches!(
        parse_ok("list-unclaimed-deposits"),
        Command::ListUnclaimedDeposits
    ));
}

#[test]
fn buy_bitcoin() {
    let Command::BuyBitcoin {
        provider,
        amount_sat,
        redirect_url,
    } = parse_ok("buy-bitcoin")
    else {
        panic!("expected BuyBitcoin");
    };
    assert_eq!(provider, "moonpay");
    assert!(amount_sat.is_none());
    assert!(redirect_url.is_none());

    let Command::BuyBitcoin {
        provider,
        amount_sat,
        redirect_url,
    } = parse_ok("buy-bitcoin --provider cashapp --amount-sat 10000 --redirect-url https://x.com")
    else {
        panic!("expected BuyBitcoin");
    };
    assert_eq!(provider, "cashapp");
    assert_eq!(amount_sat, Some(10000));
    assert_eq!(redirect_url.as_deref(), Some("https://x.com"));
}

#[test]
fn lightning_address() {
    let Command::CheckLightningAddressAvailable { username } =
        parse_ok("check-lightning-address-available alice")
    else {
        panic!("expected CheckLightningAddressAvailable");
    };
    assert_eq!(username, "alice");

    assert!(matches!(
        parse_ok("get-lightning-address"),
        Command::GetLightningAddress
    ));

    let Command::RegisterLightningAddress {
        username,
        description,
    } = parse_ok("register-lightning-address alice \"my address\"")
    else {
        panic!("expected RegisterLightningAddress");
    };
    assert_eq!(username, "alice");
    assert_eq!(description.as_deref(), Some("my address"));

    let Command::RegisterLightningAddress { description, .. } =
        parse_ok("register-lightning-address alice")
    else {
        panic!("expected RegisterLightningAddress");
    };
    assert!(description.is_none());

    assert!(matches!(
        parse_ok("delete-lightning-address"),
        Command::DeleteLightningAddress
    ));
}

#[test]
fn lightning_address_transfer() {
    let Command::AuthorizeLightningAddressTransfer { transferee_pubkey } =
        parse_ok("authorize-lightning-address-transfer 02aa")
    else {
        panic!("expected AuthorizeLightningAddressTransfer");
    };
    assert_eq!(transferee_pubkey, "02aa");

    let Command::ClaimLightningAddressTransfer {
        username,
        description,
        from_pubkey,
        from_signature,
    } = parse_ok(
        "claim-lightning-address-transfer alice --from-pubkey 02aa --from-signature 3044...",
    )
    else {
        panic!("expected ClaimLightningAddressTransfer");
    };
    assert_eq!(username, "alice");
    assert!(description.is_none());
    assert_eq!(from_pubkey, "02aa");
    assert_eq!(from_signature, "3044...");

    parse_err("claim-lightning-address-transfer alice --from-pubkey 02aa");
}

#[test]
fn fiat() {
    assert!(matches!(
        parse_ok("list-fiat-currencies"),
        Command::ListFiatCurrencies
    ));
    assert!(matches!(
        parse_ok("list-fiat-rates"),
        Command::ListFiatRates
    ));
}

#[test]
fn recommended_fees() {
    assert!(matches!(
        parse_ok("recommended-fees"),
        Command::RecommendedFees
    ));
}

#[test]
fn get_tokens_metadata() {
    let Command::GetTokensMetadata { token_identifiers } = parse_ok("get-tokens-metadata t1 t2")
    else {
        panic!("expected GetTokensMetadata");
    };
    assert_eq!(token_identifiers, vec!["t1", "t2"]);
}

#[test]
fn fetch_conversion_limits() {
    let Command::FetchConversionLimits {
        from_bitcoin,
        token_identifier,
    } = parse_ok("fetch-conversion-limits -f tok1")
    else {
        panic!("expected FetchConversionLimits");
    };
    assert!(from_bitcoin);
    assert_eq!(token_identifier, "tok1");

    let Command::FetchConversionLimits { from_bitcoin, .. } =
        parse_ok("fetch-conversion-limits tok1")
    else {
        panic!("expected FetchConversionLimits");
    };
    assert!(!from_bitcoin);
}

#[test]
fn user_settings() {
    assert!(matches!(
        parse_ok("get-user-settings"),
        Command::GetUserSettings
    ));
    let Command::SetUserSettings {
        spark_private_mode_enabled,
    } = parse_ok("set-user-settings -p true")
    else {
        panic!("expected SetUserSettings");
    };
    assert_eq!(spark_private_mode_enabled, Some(true));
}

#[test]
fn get_spark_status() {
    assert!(matches!(
        parse_ok("get-spark-status"),
        Command::GetSparkStatus
    ));
}

#[test]
fn issuer_subcommands() {
    assert!(matches!(
        parse_ok("issuer token-balance"),
        Command::Issuer(IssuerCommand::TokenBalance)
    ));
    assert!(matches!(
        parse_ok("issuer token-metadata"),
        Command::Issuer(IssuerCommand::TokenMetadata)
    ));

    let Command::Issuer(IssuerCommand::CreateToken {
        name,
        ticker,
        decimals,
        is_freezable,
        max_supply,
    }) = parse_ok("issuer create-token MyToken MTK 6 -f 1000000")
    else {
        panic!("expected CreateToken");
    };
    assert_eq!(name, "MyToken");
    assert_eq!(ticker, "MTK");
    assert_eq!(decimals, 6);
    assert!(is_freezable);
    assert_eq!(max_supply, 1_000_000);

    assert!(matches!(
        parse_ok("issuer mint-token 500"),
        Command::Issuer(IssuerCommand::MintToken { amount: 500 })
    ));
    assert!(matches!(
        parse_ok("issuer burn-token 100"),
        Command::Issuer(IssuerCommand::BurnToken { amount: 100 })
    ));
    assert!(matches!(
        parse_ok("issuer freeze-token addr1"),
        Command::Issuer(IssuerCommand::FreezeToken { .. })
    ));
    assert!(matches!(
        parse_ok("issuer unfreeze-token addr1"),
        Command::Issuer(IssuerCommand::UnfreezeToken { .. })
    ));

    parse_err("issuer");
    parse_err("issuer unknown-sub");
}

#[test]
fn contacts_subcommands() {
    let Command::Contacts(ContactCommand::Add {
        name,
        payment_identifier,
    }) = parse_ok("contacts add Alice alice@domain.com")
    else {
        panic!("expected Contacts Add");
    };
    assert_eq!(name, "Alice");
    assert_eq!(payment_identifier, "alice@domain.com");

    assert!(matches!(
        parse_ok("contacts update id1 Bob bob@domain.com"),
        Command::Contacts(ContactCommand::Update { .. })
    ));
    assert!(matches!(
        parse_ok("contacts delete id1"),
        Command::Contacts(ContactCommand::Delete { .. })
    ));
    assert!(matches!(
        parse_ok("contacts list"),
        Command::Contacts(ContactCommand::List {
            offset: None,
            limit: None
        })
    ));
    assert!(matches!(
        parse_ok("contacts list 5 20"),
        Command::Contacts(ContactCommand::List {
            offset: Some(5),
            limit: Some(20)
        })
    ));
}

#[test]
fn webhooks_subcommands() {
    let Command::Webhooks(WebhookCommand::Register {
        url,
        secret,
        events,
    }) = parse_ok("webhooks register https://hook.example s3cret lightning-receive static-deposit")
    else {
        panic!("expected Webhooks Register");
    };
    assert_eq!(url, "https://hook.example");
    assert_eq!(secret, "s3cret");
    assert!(matches!(
        events.as_slice(),
        [
            WebhookEventTypeArg::LightningReceive,
            WebhookEventTypeArg::StaticDeposit
        ]
    ));

    assert!(matches!(
        parse_ok("webhooks unregister wh1"),
        Command::Webhooks(WebhookCommand::Unregister { .. })
    ));
    assert!(matches!(
        parse_ok("webhooks list"),
        Command::Webhooks(WebhookCommand::List)
    ));

    parse_err("webhooks register https://hook.example s3cret");
    parse_err("webhooks register https://hook.example s3cret not-an-event");
}

#[test]
fn stable_balance_subcommands() {
    assert!(matches!(
        parse_ok("stable-balance get"),
        Command::StableBalance(StableBalanceCommand::Get)
    ));
    let Command::StableBalance(StableBalanceCommand::Set { label }) =
        parse_ok("stable-balance set USDB")
    else {
        panic!("expected StableBalance Set");
    };
    assert_eq!(label, "USDB");
    assert!(matches!(
        parse_ok("stable-balance unset"),
        Command::StableBalance(StableBalanceCommand::Unset)
    ));
}

#[test]
fn unknown_command() {
    let err = parse_err("definitely-not-a-command");
    assert!(
        err.contains("unrecognized subcommand"),
        "unexpected error: {err}"
    );
}
