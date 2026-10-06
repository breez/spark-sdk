use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use breez_sdk_itest::*;
use breez_sdk_spark::*;
use rstest::*;
use tracing::info;

async fn issue_token(instance: &SdkInstance) -> Result<String> {
    let issuer = instance.sdk.get_token_issuer();
    let metadata = issuer
        .create_issuer_token(CreateIssuerTokenRequest {
            name: "allowance token".to_string(),
            ticker: "AIT".to_string(),
            decimals: 0,
            is_freezable: false,
            max_supply: 1_000_000,
        })
        .await?;
    issuer
        .mint_issuer_token(MintIssuerTokenRequest { amount: 1_000_000 })
        .await?;
    tokio::time::sleep(Duration::from_secs(1)).await;
    instance.sdk.sync_wallet(SyncWalletRequest {}).await?;
    Ok(metadata.identifier)
}

async fn identity_key(instance: &SdkInstance) -> Result<String> {
    Ok(instance
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .identity_pubkey)
}

fn in_one_hour() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3600
}

fn list_request(role: TokenAllowanceRole, include_inactive: bool) -> ListTokenAllowancesRequest {
    ListTokenAllowancesRequest {
        role,
        counterparty_public_key: None,
        token_identifier: None,
        include_inactive: Some(include_inactive),
        offset: None,
        limit: None,
    }
}

async fn grant(
    owner: &SdkInstance,
    spender_key: &str,
    token_identifier: &str,
    allowed_recipients: Vec<String>,
) -> Result<TokenAllowance> {
    Ok(owner
        .sdk
        .create_token_allowance(CreateTokenAllowanceRequest {
            spender_public_key: spender_key.to_string(),
            token_identifier: token_identifier.to_string(),
            max_per_payment: TokenAllowanceLimit::Amount { amount: 5_000 },
            max_total: TokenAllowanceLimit::Amount { amount: 20_000 },
            expiry_time: in_one_hour(),
            allowed_recipients,
        })
        .await?
        .allowance)
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_01_owner_grants_lists_and_revokes(#[future] env: Result<Environment>) -> Result<()> {
    let env = env.await?;
    let owner = env.create_wallet().await?;
    let spender = env.create_wallet().await?;
    let token_identifier = issue_token(&owner).await?;
    let owner_key = identity_key(&owner).await?;
    let spender_key = identity_key(&spender).await?;

    let created = grant(&owner, &spender_key, &token_identifier, vec![]).await?;
    info!("Created allowance {}", created.id);
    assert_eq!(created.status, TokenAllowanceStatus::Active);
    assert_eq!(created.spender_public_key, spender_key);
    assert_eq!(created.owner_public_key, owner_key);
    assert_eq!(
        created.max_per_payment,
        TokenAllowanceLimit::Amount { amount: 5_000 }
    );
    assert_eq!(
        created.max_total,
        TokenAllowanceLimit::Amount { amount: 20_000 }
    );

    let owner_view = owner
        .sdk
        .list_token_allowances(list_request(TokenAllowanceRole::Owner, false))
        .await?
        .allowances;
    assert!(owner_view.iter().any(|a| a.id == created.id));

    let spender_view = spender
        .sdk
        .list_token_allowances(list_request(TokenAllowanceRole::Spender, false))
        .await?
        .allowances;
    assert!(
        spender_view
            .iter()
            .any(|a| a.id == created.id && a.owner_public_key == owner_key)
    );

    let duplicate = owner
        .sdk
        .create_token_allowance(CreateTokenAllowanceRequest {
            spender_public_key: spender_key.clone(),
            token_identifier: token_identifier.clone(),
            max_per_payment: TokenAllowanceLimit::Amount { amount: 5_000 },
            max_total: TokenAllowanceLimit::Amount { amount: 20_000 },
            expiry_time: in_one_hour(),
            allowed_recipients: vec![],
        })
        .await;
    assert!(matches!(
        duplicate,
        Err(SdkError::TokenAllowance {
            reason: TokenAllowanceErrorReason::AlreadyActive,
            ..
        })
    ));

    owner
        .sdk
        .revoke_token_allowance(RevokeTokenAllowanceRequest {
            allowance_id: created.id.clone(),
        })
        .await?;
    let after = owner
        .sdk
        .list_token_allowances(list_request(TokenAllowanceRole::Owner, true))
        .await?
        .allowances;
    let revoked = after.iter().find(|a| a.id == created.id).unwrap();
    assert_eq!(revoked.status, TokenAllowanceStatus::Revoked);
    assert!(revoked.revoked_at.is_some());
    Ok(())
}

fn to(receiver_public_key: Option<String>, amount: u128) -> PullReceiver {
    PullReceiver {
        amount,
        receiver_public_key,
    }
}

async fn prepare(
    spender: &SdkInstance,
    payer_key: &str,
    token_identifier: &str,
    receivers: Vec<PullReceiver>,
) -> Result<PreparePullPaymentResponse, SdkError> {
    spender
        .sdk
        .prepare_pull_payment(PreparePullPaymentRequest {
            payer_public_key: payer_key.to_string(),
            token_identifier: token_identifier.to_string(),
            receivers,
        })
        .await
}

async fn until_completed<F, Fut>(mut send: F) -> Result<PullPaymentResponse>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<PullPaymentResponse, SdkError>>,
{
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        match send().await {
            Ok(response) if response.status == PaymentStatus::Completed => return Ok(response),
            Ok(_) | Err(SdkError::SparkError(_) | SdkError::NetworkError(_))
                if Instant::now() < deadline => {}
            Ok(response) => anyhow::bail!("pull {} still pending after 120s", response.tx_hash),
            Err(e) => return Err(e.into()),
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

async fn pull(
    spender: &SdkInstance,
    prepare_response: &PreparePullPaymentResponse,
) -> Result<PullPaymentResponse> {
    until_completed(|| {
        spender.sdk.pull_payment(PullPaymentRequest {
            prepare_response: prepare_response.clone(),
        })
    })
    .await
}

async fn wait_for_token_send(sdk: &BreezSdk, tx_hash: &str) -> Result<Vec<Payment>> {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        sdk.sync_wallet(SyncWalletRequest {}).await?;
        let sends: Vec<Payment> = sdk
            .list_payments(ListPaymentsRequest {
                type_filter: Some(vec![PaymentType::Send]),
                ..Default::default()
            })
            .await?
            .payments
            .into_iter()
            .filter(|p| {
                matches!(&p.details, Some(PaymentDetails::Token { tx_hash: hash, .. }) if hash == tx_hash)
            })
            .collect();
        if !sends.is_empty() {
            return Ok(sends);
        }
        if Instant::now() > deadline {
            anyhow::bail!("no send with tx hash {tx_hash} after 60s");
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_02_spender_pulls_into_own_wallet_and_retries(
    #[future] env: Result<Environment>,
) -> Result<()> {
    let env = env.await?;
    let owner = env.create_wallet().await?;
    let spender = env.create_wallet().await?;
    let token_identifier = issue_token(&owner).await?;
    let owner_key = identity_key(&owner).await?;
    grant(
        &owner,
        &identity_key(&spender).await?,
        &token_identifier,
        vec![],
    )
    .await?;

    let prepared = prepare(
        &spender,
        &owner_key,
        &token_identifier,
        vec![to(None, 1_000)],
    )
    .await?;
    assert_eq!(prepared.amount, 1_000);

    let pulled = pull(&spender, &prepared).await?;
    let payment = pulled.payment.clone().expect("spender is the receiver");
    assert_eq!(payment.amount, 1_000);
    assert_eq!(payment.payment_type, PaymentType::Receive);
    wait_for_token_balance(&spender.sdk, &token_identifier, 1_000, 60).await?;

    let retried = pull(&spender, &prepared).await?;
    assert_eq!(retried.tx_hash, pulled.tx_hash);
    wait_for_token_balance(&spender.sdk, &token_identifier, 1_000, 30).await?;

    let sends = wait_for_token_send(&owner.sdk, &pulled.tx_hash).await?;
    assert_eq!(sends.len(), 1);
    assert_eq!(sends[0].amount, 1_000);
    Ok(())
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_03_one_pull_pays_several_receivers(#[future] env: Result<Environment>) -> Result<()> {
    let env = env.await?;
    let owner = env.create_wallet().await?;
    let spender = env.create_wallet().await?;
    let seller = env.create_wallet().await?;
    let platform = env.create_wallet().await?;
    let token_identifier = issue_token(&owner).await?;
    let owner_key = identity_key(&owner).await?;
    grant(
        &owner,
        &identity_key(&spender).await?,
        &token_identifier,
        vec![],
    )
    .await?;

    let prepared = prepare(
        &spender,
        &owner_key,
        &token_identifier,
        vec![
            to(Some(identity_key(&seller).await?), 950),
            to(Some(identity_key(&platform).await?), 50),
        ],
    )
    .await?;
    let pulled = pull(&spender, &prepared).await?;
    assert!(pulled.payment.is_none());
    wait_for_token_balance(&seller.sdk, &token_identifier, 950, 60).await?;
    wait_for_token_balance(&platform.sdk, &token_identifier, 50, 60).await?;
    let sends = wait_for_token_send(&owner.sdk, &pulled.tx_hash).await?;
    assert_eq!(sends.len(), 2);

    let allowances = owner
        .sdk
        .list_token_allowances(list_request(TokenAllowanceRole::Owner, false))
        .await?
        .allowances;
    assert_eq!(allowances[0].spent_amount, 1_000);
    Ok(())
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_04_allowlist_limits_receivers(#[future] env: Result<Environment>) -> Result<()> {
    let env = env.await?;
    let owner = env.create_wallet().await?;
    let spender = env.create_wallet().await?;
    let settlement = env.create_wallet().await?;
    let token_identifier = issue_token(&owner).await?;
    let owner_key = identity_key(&owner).await?;
    let settlement_key = identity_key(&settlement).await?;
    grant(
        &owner,
        &identity_key(&spender).await?,
        &token_identifier,
        vec![settlement_key.clone()],
    )
    .await?;

    let into_spender = prepare(&spender, &owner_key, &token_identifier, vec![to(None, 100)]).await;
    assert!(matches!(
        into_spender,
        Err(SdkError::TokenAllowance {
            reason: TokenAllowanceErrorReason::RecipientNotAllowed,
            ..
        })
    ));

    let prepared = prepare(
        &spender,
        &owner_key,
        &token_identifier,
        vec![to(Some(settlement_key), 100)],
    )
    .await?;
    pull(&spender, &prepared).await?;
    wait_for_token_balance(&settlement.sdk, &token_identifier, 100, 60).await?;
    Ok(())
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_05_pull_fails_after_payer_spends(#[future] env: Result<Environment>) -> Result<()> {
    let env = env.await?;
    let owner = env.create_wallet().await?;
    let spender = env.create_wallet().await?;
    let sink = env.create_wallet().await?;
    let token_identifier = issue_token(&owner).await?;
    let owner_key = identity_key(&owner).await?;
    grant(
        &owner,
        &identity_key(&spender).await?,
        &token_identifier,
        vec![],
    )
    .await?;

    let old = prepare(&spender, &owner_key, &token_identifier, vec![to(None, 100)]).await?;

    let sink_address = sink
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;
    let send = owner
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: PaymentRequest::Input {
                input: sink_address,
            },
            amount: Some(10),
            token_identifier: Some(token_identifier.clone()),
            conversion_options: None,
            fee_policy: None,
        })
        .await?;
    owner
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: send,
            options: None,
            idempotency_key: None,
        })
        .await?;

    let error = spender
        .sdk
        .pull_payment(PullPaymentRequest {
            prepare_response: old,
        })
        .await
        .expect_err("the payer spent the tokens the pull was prepared with");
    assert!(
        matches!(
            error,
            SdkError::TokenAllowance {
                reason: TokenAllowanceErrorReason::PreparedPullStale,
                ..
            }
        ),
        "{error}"
    );

    let fresh = prepare(&spender, &owner_key, &token_identifier, vec![to(None, 100)]).await?;
    pull(&spender, &fresh).await?;
    wait_for_token_balance(&spender.sdk, &token_identifier, 100, 60).await?;
    Ok(())
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_06_refusals(#[future] env: Result<Environment>) -> Result<()> {
    let env = env.await?;
    let owner = env.create_wallet().await?;
    let spender = env.create_wallet().await?;
    let token_identifier = issue_token(&owner).await?;
    let owner_key = identity_key(&owner).await?;
    let allowance = grant(
        &owner,
        &identity_key(&spender).await?,
        &token_identifier,
        vec![],
    )
    .await?;

    let over_cap = prepare(
        &spender,
        &owner_key,
        &token_identifier,
        vec![to(None, 6_000)],
    )
    .await;
    assert!(matches!(
        over_cap,
        Err(SdkError::TokenAllowance {
            reason: TokenAllowanceErrorReason::OverPerPaymentLimit,
            ..
        })
    ));

    owner
        .sdk
        .revoke_token_allowance(RevokeTokenAllowanceRequest {
            allowance_id: allowance.id,
        })
        .await?;
    let after_revoke = prepare(&spender, &owner_key, &token_identifier, vec![to(None, 100)]).await;
    assert!(matches!(
        after_revoke,
        Err(SdkError::TokenAllowance {
            reason: TokenAllowanceErrorReason::Revoked,
            ..
        })
    ));
    Ok(())
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_07_client_signed_pull(#[future] env: Result<Environment>) -> Result<()> {
    let env = env.await?;
    let owner = env.create_wallet().await?;
    let token_identifier = issue_token(&owner).await?;
    let owner_key = identity_key(&owner).await?;

    let spender_mnemonic = fixtures::random_mnemonic()?;
    let dir = tempfile::Builder::new()
        .prefix("breez-sdk-allowance-client-signing")
        .tempdir()?;
    let path = dir.path().to_string_lossy().to_string();
    let spender = build_sdk_with_external_signer(path, spender_mnemonic.clone(), Some(dir)).await?;
    let client_signer =
        default_external_signers(spender_mnemonic, None, Network::Regtest, None)?.spark_signer;
    grant(
        &owner,
        &identity_key(&spender).await?,
        &token_identifier,
        vec![],
    )
    .await?;

    let prepared = prepare(&spender, &owner_key, &token_identifier, vec![to(None, 700)]).await?;
    let unsigned = spender
        .sdk
        .build_unsigned_pull_package(BuildUnsignedPullPackageRequest {
            prepare_response: prepared,
        })
        .await?;
    let UnsignedTransferPackage::TokenPull {
        prepare_token_transaction,
        ..
    } = &unsigned
    else {
        panic!("expected a TokenPull package");
    };
    let signed = client_signer
        .prepare_token_transaction(prepare_token_transaction.clone())
        .await?;
    let signed_package = SignedTransferPackage {
        unsigned,
        signature: TransferSignature::Token { signed },
    };

    let publish = || {
        spender
            .sdk
            .publish_signed_pull_package(PublishSignedPullPackageRequest {
                signed_package: signed_package.clone(),
            })
    };
    let published = until_completed(publish).await?;
    assert_eq!(published.payment.as_ref().map(|p| p.amount), Some(700));

    let republished = until_completed(publish).await?;
    assert_eq!(republished.tx_hash, published.tx_hash);
    wait_for_token_balance(&spender.sdk, &token_identifier, 700, 60).await?;
    Ok(())
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_08_operators_refuse_pull_over_total_limit(
    #[future] env: Result<Environment>,
) -> Result<()> {
    let env = env.await?;
    let owner = env.create_wallet().await?;
    let spender = env.create_wallet().await?;
    let token_identifier = issue_token(&owner).await?;
    let owner_key = identity_key(&owner).await?;
    owner
        .sdk
        .create_token_allowance(CreateTokenAllowanceRequest {
            spender_public_key: identity_key(&spender).await?,
            token_identifier: token_identifier.clone(),
            max_per_payment: TokenAllowanceLimit::Amount { amount: 1_000 },
            max_total: TokenAllowanceLimit::Amount { amount: 1_500 },
            expiry_time: in_one_hour(),
            allowed_recipients: vec![],
        })
        .await?;

    let first = prepare(
        &spender,
        &owner_key,
        &token_identifier,
        vec![to(None, 1_000)],
    )
    .await?;
    pull(&spender, &first).await?;
    wait_for_token_balance(&spender.sdk, &token_identifier, 1_000, 60).await?;

    let second = prepare(
        &spender,
        &owner_key,
        &token_identifier,
        vec![to(None, 1_000)],
    )
    .await?;
    let result = spender
        .sdk
        .pull_payment(PullPaymentRequest {
            prepare_response: second,
        })
        .await;
    assert!(matches!(
        result,
        Err(SdkError::TokenAllowance {
            reason: TokenAllowanceErrorReason::OverTotalLimit,
            ..
        })
    ));
    Ok(())
}
