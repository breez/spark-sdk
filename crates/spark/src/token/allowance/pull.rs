use std::collections::HashSet;

use bitcoin::secp256k1::{PublicKey, schnorr};
use platform_utils::time::{SystemTime, UNIX_EPOCH};
use prost::Message as _;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    Network,
    address::SparkAddress,
    operator::rpc::{
        multisig::KeyedSignature,
        spark_token::{
            AllowanceSignature, BroadcastTransactionRequest, PartialTokenTransaction,
            SignatureWithIndex, partial_token_transaction::TokenInputs,
            signature_with_index::AuthoritySignatures,
        },
    },
    services::ServiceError,
    token::{
        MAX_TOKEN_TX_OUTPUTS, TokenAllowance, TokenAllowanceFailure, TokenAllowanceStatus,
        TokenOutputWithPrevOut, TokensConfig, bech32m_decode_token_id, token_service::unix_micros,
    },
};

use super::hash::hash_delegated_spend;

const PULL_VALIDITY_SECONDS: u64 = 300;
const MAX_PULL_RECEIVERS: usize = MAX_TOKEN_TX_OUTPUTS - 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullReceiver {
    pub receiver_public_key: PublicKey,
    pub amount: u128,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PreparedTokenPull {
    pub allowance_id: String,
    pub payer_public_key: PublicKey,
    pub token_identifier: String,
    pub receivers: Vec<PullReceiver>,
    pub partial_token_transaction_bytes: Vec<u8>,
    pub partial_token_transaction_hash: Vec<u8>,
    pub created_timestamp: SystemTime,
    pub allowance_expiry_time: u64,
}

impl PreparedTokenPull {
    pub fn total(&self) -> u128 {
        self.receivers
            .iter()
            .fold(0, |total, r| total.saturating_add(r.amount))
    }

    pub fn expiry_time(&self) -> u64 {
        self.window_end().min(self.allowance_expiry_time)
    }

    pub(crate) fn window_end(&self) -> u64 {
        self.created_timestamp
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs())
            + PULL_VALIDITY_SECONDS
    }

    pub fn spend_digest(&self) -> Result<[u8; 32], ServiceError> {
        let allowance_id = Uuid::parse_str(&self.allowance_id)
            .map_err(|e| ServiceError::InvalidInput(format!("invalid allowance id: {e}")))?;
        hash_delegated_spend(
            &self.partial_token_transaction_hash,
            allowance_id.as_bytes(),
        )
    }
}

pub(crate) struct PullBuildParams<'a> {
    pub allowance_id: String,
    pub allowance_expiry_time: u64,
    pub payer: PublicKey,
    pub token_identifier: &'a str,
    pub receivers: Vec<PullReceiver>,
    pub selected: Vec<TokenOutputWithPrevOut>,
    pub network: Network,
    pub operator_identity_public_keys: Vec<Vec<u8>>,
    pub tokens_config: &'a TokensConfig,
    pub now: SystemTime,
}

pub(crate) fn validate_pull_receivers(
    receivers: &[PullReceiver],
    payer: PublicKey,
) -> Result<u128, ServiceError> {
    if receivers.is_empty() {
        return Err(ServiceError::InvalidInput(
            "a pull needs at least one receiver".to_string(),
        ));
    }
    if receivers.len() > MAX_PULL_RECEIVERS {
        return Err(ServiceError::InvalidInput(format!(
            "a pull is limited to {MAX_PULL_RECEIVERS} receivers"
        )));
    }
    let mut seen = HashSet::new();
    let mut total: u128 = 0;
    for receiver in receivers {
        if receiver.amount == 0 {
            return Err(ServiceError::InvalidInput(
                "receiver amounts must be positive".to_string(),
            ));
        }
        if receiver.receiver_public_key == payer {
            return Err(ServiceError::InvalidInput(
                "the payer can't be a receiver".to_string(),
            ));
        }
        if !seen.insert(receiver.receiver_public_key) {
            return Err(ServiceError::InvalidInput(
                "receivers must be distinct".to_string(),
            ));
        }
        total = total
            .checked_add(receiver.amount)
            .ok_or_else(|| ServiceError::InvalidInput("pull total overflows".to_string()))?;
    }
    Ok(total)
}

pub(crate) fn check_pull_against_allowance(
    allowance: &TokenAllowance,
    receivers: &[PullReceiver],
    now_secs: u64,
) -> Result<(), ServiceError> {
    match allowance.status {
        TokenAllowanceStatus::Revoked => {
            return Err(
                TokenAllowanceFailure::Revoked.into_error("the token allowance was revoked")
            );
        }
        TokenAllowanceStatus::Expired => {
            return Err(
                TokenAllowanceFailure::Expired.into_error("the token allowance has expired")
            );
        }
        TokenAllowanceStatus::Active | TokenAllowanceStatus::Exhausted => {}
    }
    if allowance.expiry_time <= now_secs {
        return Err(TokenAllowanceFailure::Expired.into_error("the token allowance has expired"));
    }
    let total: u128 = receivers.iter().map(|r| r.amount).sum();
    if let Some(cap) = allowance.max_per_payment
        && total > cap
    {
        return Err(TokenAllowanceFailure::OverPerPaymentLimit
            .into_error("the amount exceeds the per-payment limit"));
    }
    if !allowance.allowed_recipients.is_empty()
        && receivers.iter().any(|r| {
            !allowance
                .allowed_recipients
                .contains(&r.receiver_public_key)
        })
    {
        return Err(TokenAllowanceFailure::RecipientNotAllowed
            .into_error("a receiver is not on the allowance's recipient list"));
    }
    Ok(())
}

pub(crate) fn pull_candidates(
    free: Vec<TokenOutputWithPrevOut>,
    pending: Vec<TokenOutputWithPrevOut>,
    total: u128,
) -> Vec<TokenOutputWithPrevOut> {
    let free_total = free
        .iter()
        .fold(0u128, |sum, o| sum.saturating_add(o.output.token_amount));
    if free_total >= total {
        free
    } else {
        free.into_iter().chain(pending).collect()
    }
}

pub(crate) fn build_pull(params: PullBuildParams<'_>) -> Result<PreparedTokenPull, ServiceError> {
    let PullBuildParams {
        allowance_id,
        allowance_expiry_time,
        payer,
        token_identifier,
        receivers,
        mut selected,
        network,
        operator_identity_public_keys,
        tokens_config,
        now,
    } = params;
    if selected.len() > tokens_config.max_tx_inputs {
        return Err(ServiceError::NeededTooManyOutputs);
    }
    selected.sort_by_key(|o| o.prev_tx_vout);
    let raw_token_id = bech32m_decode_token_id(token_identifier, Some(network))?;
    let selected_outputs = selected
        .iter()
        .map(|o| {
            Ok(spark_primitives::SelectedTokenOutput {
                previous_transaction_hash: hex::decode(&o.prev_tx_hash)
                    .map_err(|_| ServiceError::Generic("invalid prev tx hash".to_string()))?,
                previous_transaction_vout: o.prev_tx_vout,
                owner_public_key: o.output.owner_public_key.serialize().to_vec(),
                token_identifier: raw_token_id.clone(),
                token_amount: o.output.token_amount.to_be_bytes().to_vec(),
            })
        })
        .collect::<Result<Vec<_>, ServiceError>>()?;
    let receiver_outputs = receivers
        .iter()
        .map(|r| {
            Ok(spark_primitives::ReceiverTokenOutput {
                receiver_spark_address: SparkAddress::new(r.receiver_public_key, network, None)
                    .to_address_string()
                    .map_err(|e| ServiceError::Generic(e.to_string()))?,
                token_identifier: Some(raw_token_id.clone()),
                token_amount: Some(r.amount.to_be_bytes().to_vec()),
            })
        })
        .collect::<Result<Vec<_>, ServiceError>>()?;
    let proto_network = u32::try_from(network.to_proto_network() as i32)
        .map_err(|_| ServiceError::Generic("network proto value is negative".to_string()))?;
    let built = spark_primitives::construct_partial_transfer_transaction(
        spark_primitives::TransferBuildRequest {
            identity_public_key: payer.serialize().to_vec(),
            selected_outputs,
            receiver_outputs,
            operator_identity_public_keys,
            network: proto_network,
            validity_duration_seconds: PULL_VALIDITY_SECONDS,
            client_created_timestamp_unix_micros: unix_micros(now)?,
            withdraw_bond_sats: tokens_config.expected_withdraw_bond_sats,
            withdraw_relative_block_locktime: tokens_config
                .expected_withdraw_relative_block_locktime,
            execute_before_unix_micros: None,
        },
    )
    .map_err(|e| ServiceError::Generic(e.to_string()))?;
    Ok(PreparedTokenPull {
        allowance_id,
        payer_public_key: payer,
        token_identifier: token_identifier.to_string(),
        receivers,
        partial_token_transaction_bytes: built.partial_token_transaction_bytes,
        partial_token_transaction_hash: built.partial_token_transaction_hash,
        created_timestamp: now,
        allowance_expiry_time,
    })
}

pub(crate) fn build_pull_broadcast_request(
    prepared: &PreparedTokenPull,
    spender: PublicKey,
    signature: &schnorr::Signature,
) -> Result<BroadcastTransactionRequest, ServiceError> {
    let partial =
        PartialTokenTransaction::decode(prepared.partial_token_transaction_bytes.as_slice())
            .map_err(|e| ServiceError::Generic(format!("invalid pull transaction: {e}")))?;
    let input_count = match &partial.token_inputs {
        Some(TokenInputs::TransferInput(transfer)) => transfer.outputs_to_spend.len(),
        _ => {
            return Err(ServiceError::Generic(
                "a pull transaction must spend transfer inputs".to_string(),
            ));
        }
    };
    let allowance_id = Uuid::parse_str(&prepared.allowance_id)
        .map_err(|e| ServiceError::InvalidInput(format!("invalid allowance id: {e}")))?
        .as_bytes()
        .to_vec();
    let spender_bytes = spender.serialize().to_vec();
    let signature_bytes = signature.serialize().to_vec();
    let signatures = (0..input_count)
        .map(|index| {
            Ok(SignatureWithIndex {
                signature: None,
                input_index: u32::try_from(index)
                    .map_err(|_| ServiceError::Generic("too many inputs".to_string()))?,
                authority_signatures: Some(AuthoritySignatures::AllowanceSignature(
                    AllowanceSignature {
                        allowance_id: allowance_id.clone(),
                        spender_signature: Some(KeyedSignature {
                            public_key: spender_bytes.clone(),
                            signature: signature_bytes.clone(),
                        }),
                    },
                )),
            })
        })
        .collect::<Result<Vec<_>, ServiceError>>()?;
    Ok(BroadcastTransactionRequest {
        identity_public_key: spender_bytes,
        partial_token_transaction: Some(partial),
        token_transaction_owner_signatures: signatures,
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
    use macros::test_all;
    use platform_utils::time::{SystemTime, UNIX_EPOCH};
    use prost::Message as _;
    use uuid::Uuid;

    use super::{
        PULL_VALIDITY_SECONDS, PreparedTokenPull, PullBuildParams, PullReceiver, build_pull,
        check_pull_against_allowance, pull_candidates, validate_pull_receivers,
    };
    use crate::{
        Network,
        operator::rpc::spark_token::PartialTokenTransaction,
        services::ServiceError,
        token::{
            DEFAULT_MAX_TOKEN_TX_INPUTS, MAX_TOKEN_TX_OUTPUTS, TokenAllowance,
            TokenAllowanceFailure, TokenAllowanceStatus, TokenOutput, TokenOutputWithPrevOut,
            TokensConfig, bech32m_encode_token_id,
        },
    };

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    fn key(fill: u8) -> PublicKey {
        PublicKey::from_secret_key(
            &Secp256k1::new(),
            &SecretKey::from_slice(&[fill; 32]).unwrap(),
        )
    }

    fn token_id() -> String {
        bech32m_encode_token_id(&[7; 32], Network::Regtest).unwrap()
    }

    fn output(owner: PublicKey, amount: u128, vout: u32) -> TokenOutputWithPrevOut {
        TokenOutputWithPrevOut {
            output: TokenOutput {
                owner_public_key: owner,
                revocation_commitment: String::new(),
                withdraw_bond_sats: 10_000,
                withdraw_relative_block_locktime: 1_000,
                token_public_key: None,
                token_identifier: token_id(),
                token_amount: amount,
            },
            prev_tx_hash: "ab".repeat(32),
            prev_tx_vout: vout,
        }
    }

    #[test_all]
    fn a_pull_prefers_outputs_that_are_not_being_spent() {
        let free = vec![output(key(1), 60, 0), output(key(1), 50, 1)];
        let pending = vec![output(key(1), 10, 2)];
        assert_eq!(pull_candidates(free.clone(), pending.clone(), 100), free);
        assert_eq!(
            pull_candidates(free.clone(), pending.clone(), 120),
            [free, pending].concat()
        );
    }

    fn allowance() -> TokenAllowance {
        TokenAllowance {
            id: Uuid::now_v7().to_string(),
            owner_public_key: key(1),
            spender_public_key: key(2),
            token_identifier: token_id(),
            max_per_payment: Some(500),
            max_total: Some(1_000),
            spent_amount: 0,
            allowed_recipients: vec![],
            expiry_time: 2_000_000_000,
            created_at: 1_700_000_000,
            revoked_at: None,
            status: TokenAllowanceStatus::Active,
        }
    }

    fn failure(error: ServiceError) -> Option<TokenAllowanceFailure> {
        match error {
            ServiceError::TokenAllowance { failure, .. } => Some(failure),
            _ => None,
        }
    }

    #[test_all]
    fn builds_pull_with_change_to_payer() {
        let payer = key(1);
        let receiver = key(3);
        let prepared = build_pull(PullBuildParams {
            allowance_id: Uuid::now_v7().to_string(),
            allowance_expiry_time: 2_000_000_000,
            payer,
            token_identifier: &token_id(),
            receivers: vec![PullReceiver {
                receiver_public_key: receiver,
                amount: 100,
            }],
            selected: vec![output(payer, 250, 0)],
            network: Network::Regtest,
            operator_identity_public_keys: vec![
                key(4).serialize().to_vec(),
                key(5).serialize().to_vec(),
            ],
            tokens_config: &TokensConfig {
                expected_withdraw_bond_sats: 10_000,
                expected_withdraw_relative_block_locktime: 1_000,
                transaction_validity_duration_seconds: 180,
                max_tx_inputs: DEFAULT_MAX_TOKEN_TX_INPUTS,
            },
            now: SystemTime::now(),
        })
        .unwrap();

        let partial =
            PartialTokenTransaction::decode(prepared.partial_token_transaction_bytes.as_slice())
                .unwrap();
        let owners: Vec<Vec<u8>> = partial
            .partial_token_outputs
            .iter()
            .map(|o| o.owner_public_key.clone())
            .collect();
        assert_eq!(
            owners,
            vec![receiver.serialize().to_vec(), payer.serialize().to_vec()]
        );
        assert_eq!(
            partial
                .token_transaction_metadata
                .unwrap()
                .validity_duration_seconds,
            PULL_VALIDITY_SECONDS
        );
        assert_eq!(prepared.total(), 100);
        assert_eq!(prepared.spend_digest().unwrap().len(), 32);
    }

    #[test_all]
    fn a_pull_expires_with_its_window_or_its_allowance() {
        let pull = |allowance_expiry_time| PreparedTokenPull {
            allowance_id: Uuid::now_v7().to_string(),
            payer_public_key: key(1),
            token_identifier: token_id(),
            receivers: vec![],
            partial_token_transaction_bytes: vec![],
            partial_token_transaction_hash: vec![],
            created_timestamp: UNIX_EPOCH + Duration::from_secs(1_000),
            allowance_expiry_time,
        };
        assert_eq!(pull(2_000).expiry_time(), 1_000 + PULL_VALIDITY_SECONDS);
        assert_eq!(pull(1_100).expiry_time(), 1_100);
    }

    #[test_all]
    fn rejects_bad_receivers() {
        let payer = key(1);
        let receiver = |fill: u8, amount: u128| PullReceiver {
            receiver_public_key: key(fill),
            amount,
        };
        assert!(validate_pull_receivers(&[], payer).is_err());
        assert!(validate_pull_receivers(&[receiver(3, 0)], payer).is_err());
        assert!(validate_pull_receivers(&[receiver(1, 10)], payer).is_err());
        assert!(validate_pull_receivers(&[receiver(3, 10), receiver(3, 5)], payer).is_err());
        assert!(validate_pull_receivers(&[receiver(3, 10), receiver(4, 5)], payer).is_ok());
    }

    #[test_all]
    fn caps_receivers_to_leave_room_for_change() {
        let payer = key(1);
        let receivers = |count: u32| {
            (0..count)
                .map(|index| {
                    let mut secret = [7; 32];
                    secret[28..].copy_from_slice(&index.to_be_bytes());
                    PullReceiver {
                        receiver_public_key: PublicKey::from_secret_key(
                            &Secp256k1::new(),
                            &SecretKey::from_slice(&secret).unwrap(),
                        ),
                        amount: 1,
                    }
                })
                .collect::<Vec<_>>()
        };
        let cap = u32::try_from(MAX_TOKEN_TX_OUTPUTS - 1).unwrap();
        assert!(validate_pull_receivers(&receivers(cap), payer).is_ok());
        assert!(matches!(
            validate_pull_receivers(&receivers(cap + 1), payer),
            Err(ServiceError::InvalidInput(_))
        ));
    }

    #[test_all]
    fn checks_pull_against_allowance() {
        let to = |fill: u8, amount: u128| {
            vec![PullReceiver {
                receiver_public_key: key(fill),
                amount,
            }]
        };
        assert!(check_pull_against_allowance(&allowance(), &to(3, 500), 1_800_000_000).is_ok());
        assert_eq!(
            failure(
                check_pull_against_allowance(&allowance(), &to(3, 501), 1_800_000_000).unwrap_err()
            ),
            Some(TokenAllowanceFailure::OverPerPaymentLimit)
        );
        let listed = TokenAllowance {
            allowed_recipients: vec![key(4)],
            ..allowance()
        };
        assert_eq!(
            failure(check_pull_against_allowance(&listed, &to(3, 10), 1_800_000_000).unwrap_err()),
            Some(TokenAllowanceFailure::RecipientNotAllowed)
        );
        assert_eq!(
            failure(
                check_pull_against_allowance(&allowance(), &to(3, 10), 2_000_000_000).unwrap_err()
            ),
            Some(TokenAllowanceFailure::Expired)
        );
        let revoked = TokenAllowance {
            status: TokenAllowanceStatus::Revoked,
            ..allowance()
        };
        assert_eq!(
            failure(check_pull_against_allowance(&revoked, &to(3, 10), 1_800_000_000).unwrap_err()),
            Some(TokenAllowanceFailure::Revoked)
        );
    }

    #[test_all]
    fn broadcast_request_carries_allowance_signatures() {
        use bitcoin::secp256k1::{Keypair, Message};

        use super::build_pull_broadcast_request;
        use crate::operator::rpc::spark_token::signature_with_index::AuthoritySignatures;

        let payer = key(1);
        let secp = Secp256k1::new();
        let spender_secret = SecretKey::from_slice(&[9; 32]).unwrap();
        let spender = PublicKey::from_secret_key(&secp, &spender_secret);
        let allowance_id = Uuid::now_v7();
        let prepared = build_pull(PullBuildParams {
            allowance_id: allowance_id.to_string(),
            allowance_expiry_time: 2_000_000_000,
            payer,
            token_identifier: &token_id(),
            receivers: vec![PullReceiver {
                receiver_public_key: spender,
                amount: 300,
            }],
            selected: vec![output(payer, 200, 0), output(payer, 200, 1)],
            network: Network::Regtest,
            operator_identity_public_keys: vec![key(4).serialize().to_vec()],
            tokens_config: &TokensConfig {
                expected_withdraw_bond_sats: 10_000,
                expected_withdraw_relative_block_locktime: 1_000,
                transaction_validity_duration_seconds: 180,
                max_tx_inputs: DEFAULT_MAX_TOKEN_TX_INPUTS,
            },
            now: SystemTime::now(),
        })
        .unwrap();
        let signature = secp.sign_schnorr_no_aux_rand(
            &Message::from_digest(prepared.spend_digest().unwrap()),
            &Keypair::from_secret_key(&secp, &spender_secret),
        );

        let request = build_pull_broadcast_request(&prepared, spender, &signature).unwrap();

        assert_eq!(request.identity_public_key, spender.serialize().to_vec());
        assert_eq!(request.token_transaction_owner_signatures.len(), 2);
        for (index, entry) in request
            .token_transaction_owner_signatures
            .iter()
            .enumerate()
        {
            assert_eq!(entry.input_index as usize, index);
            let Some(AuthoritySignatures::AllowanceSignature(allowance_signature)) =
                &entry.authority_signatures
            else {
                panic!("expected an allowance signature");
            };
            assert_eq!(
                allowance_signature.allowance_id,
                allowance_id.as_bytes().to_vec()
            );
            let keyed = allowance_signature.spender_signature.as_ref().unwrap();
            assert_eq!(keyed.public_key, spender.serialize().to_vec());
            assert_eq!(keyed.signature, signature.serialize().to_vec());
        }
    }
}
