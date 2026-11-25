use std::sync::Arc;

use bitcoin::{
    bech32::{self, Bech32m, Hrp},
    secp256k1::PublicKey,
};
use platform_utils::time::SystemTime;
use serde::{Deserialize, Serialize};

use crate::{
    Network,
    operator::{
        OperatorPool,
        rpc::{
            self,
            spark_token::{
                QueryTokenMetadataRequest, QueryTokenTransactionsByFilters,
                QueryTokenTransactionsByTxHash, QueryTokenTransactionsRequest,
                query_token_transactions_request::QueryType,
            },
        },
    },
    services::{
        QueryTokenTransactionsFilter, ServiceError, TokenInputs, TokenTransaction,
        TransferObserver, convert_page,
    },
    signer::SparkSigner,
    token::{
        GetTokenOutputsFilter, TokenMetadata, TokenOutputService, TokenOutputWithPrevOut,
        TokenOutputs,
    },
    utils::paging::{PagingFilter, PagingResult, pager},
};

/// Default cap on the number of inputs a single token transaction may spend.
/// Overridable per wallet via [`TokensConfig::max_tx_inputs`].
pub const DEFAULT_MAX_TOKEN_TX_INPUTS: usize = 500;
/// Cap on the outputs a single token transaction may create, counting the change
/// output each token with a remainder gets. Mirrors the constant the transaction
/// builder enforces, which `spark-primitives` keeps private: exceeding it
/// fails deep in construction, so callers sizing a transfer check it up front.
pub const MAX_TOKEN_TX_OUTPUTS: usize = 500;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TokenOutpoint {
    pub prev_tx_hash: String,
    pub prev_tx_vout: u32,
    /// Identifier of the token this outpoint holds. A transfer may spend outpoints
    /// of several tokens, so it cannot be carried once for the whole transaction.
    pub token_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PreparedTokenReceiverOutput {
    pub token_id: String,
    pub amount: u128,
    pub pay_request: String,
    pub spark_invoice: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PreparedTokenTransfer {
    pub partial_token_transaction_hash: Vec<u8>,
    pub partial_token_transaction_bytes: Vec<u8>,
    pub spent_outpoints: Vec<TokenOutpoint>,
    pub receiver_outputs: Vec<PreparedTokenReceiverOutput>,
    pub created_timestamp: SystemTime,
}

/// Outcome of preparing a token transfer for external signing.
///
/// The client-signing flow cannot self-consolidate (that needs the external
/// signer), so when a send would exceed the per-transaction input cap the SDK
/// returns a `Consolidation` self-transfer for the client to sign and publish;
/// the caller then rebuilds and retries, mirroring the sats denomination-swap
/// step.
pub enum PreparedTokenPackage {
    Ready(PreparedTokenTransfer),
    Consolidation(PreparedTokenTransfer),
}

const HRP_STR_MAINNET: &str = "btkn";
const HRP_STR_TESTNET: &str = "btknt";
const HRP_STR_REGTEST: &str = "btknrt";
const HRP_STR_SIGNET: &str = "btkns";

pub const BURN_PUBLIC_KEY: &[u8; 33] = &[2; 33];

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TokensConfig {
    pub expected_withdraw_bond_sats: u64,
    pub expected_withdraw_relative_block_locktime: u64,
    pub transaction_validity_duration_seconds: u64,
    /// Cap on the inputs a single token transaction may spend. A send needing
    /// more triggers a consolidation instead. Defaults to
    /// [`DEFAULT_MAX_TOKEN_TX_INPUTS`].
    pub max_tx_inputs: usize,
}

#[cfg_attr(not(feature = "tokens"), allow(dead_code))]
pub struct TokenService {
    pub(super) token_output_service: Arc<dyn TokenOutputService>,
    pub(super) spark_signer: Arc<dyn SparkSigner>,
    pub(super) operator_pool: Arc<OperatorPool>,
    pub(super) network: Network,
    pub(super) tokens_config: TokensConfig,
    pub(super) transfer_observer: Option<Arc<dyn TransferObserver>>,
}

impl TokenService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        token_output_service: Arc<dyn TokenOutputService>,
        spark_signer: Arc<dyn SparkSigner>,
        operator_pool: Arc<OperatorPool>,
        network: Network,
        tokens_config: TokensConfig,
        transfer_observer: Option<Arc<dyn TransferObserver>>,
    ) -> Self {
        Self {
            token_output_service,
            spark_signer,
            operator_pool,
            network,
            tokens_config,
            transfer_observer,
        }
    }

    /// Updates the local token output store to reflect a completed token
    /// transaction: removes spent inputs and inserts any new outputs that
    /// belong to `identity_public_key`.
    ///
    /// This is the single entry-point for reconciling the local output pool
    /// after receiving a token-transaction event (including transactions
    /// initiated on another device).
    pub async fn update_token_outputs_for_transaction(
        &self,
        transaction: &TokenTransaction,
        identity_public_key: &PublicKey,
    ) -> Result<(), ServiceError> {
        // 1. Collect spent input refs.
        let spent_refs: Vec<(String, u32)> =
            if let TokenInputs::Transfer(ref transfer_input) = transaction.inputs {
                transfer_input
                    .outputs_to_spend
                    .iter()
                    .map(|o| (o.prev_token_tx_hash.clone(), o.prev_token_tx_vout))
                    .collect()
            } else {
                Vec::new()
            };

        // 2. Collect outputs that belong to us.
        let our_outputs: Vec<TokenOutputWithPrevOut> = transaction
            .outputs
            .iter()
            .enumerate()
            .filter(|(_, o)| o.owner_public_key == *identity_public_key)
            .map(|(vout, output)| TokenOutputWithPrevOut {
                output: output.clone(),
                prev_tx_hash: transaction.hash.clone(),
                prev_tx_vout: vout as u32,
            })
            .collect();

        // 3. Bundle our outputs per token, each with its own metadata.
        let mut token_ids: Vec<&str> = our_outputs
            .iter()
            .map(|o| o.output.token_identifier.as_str())
            .collect();
        token_ids.sort_unstable();
        token_ids.dedup();

        let outputs_to_add = TokenOutputs {
            metadata: self.get_tokens_metadata(&token_ids, &[]).await?,
            outputs: our_outputs,
        };

        // 4. Atomically remove spent inputs and insert new outputs.
        self.token_output_service
            .update_token_outputs(&spent_refs, &outputs_to_add)
            .await?;

        Ok(())
    }

    /// Refreshes token outputs from the operator.
    pub async fn refresh_tokens_outputs(&self) -> Result<(), ServiceError> {
        self.token_output_service.refresh_tokens_outputs().await?;
        Ok(())
    }

    /// Returns the metadata for the given token identifiers.
    ///
    /// For token identifiers that are not found in the local cache, the metadata will be queried from the SE.
    pub async fn get_tokens_metadata(
        &self,
        token_identifiers: &[&str],
        issuer_public_keys: &[PublicKey],
    ) -> Result<Vec<TokenMetadata>, ServiceError> {
        // Separate token identifiers into cached and uncached
        let mut cached_metadata = Vec::new();
        let mut uncached_identifiers = Vec::new();
        let mut uncached_issuer_public_keys = Vec::new();

        for token_id in token_identifiers {
            if let Ok(metadata) = self
                .token_output_service
                .get_token_metadata(GetTokenOutputsFilter::Identifier(token_id))
                .await
            {
                cached_metadata.push(metadata);
            } else {
                uncached_identifiers.push(*token_id);
            }
        }

        for issuer_pk in issuer_public_keys {
            if let Ok(metadata) = self
                .token_output_service
                .get_token_metadata(GetTokenOutputsFilter::IssuerPublicKey(issuer_pk))
                .await
            {
                cached_metadata.push(metadata);
            } else {
                uncached_issuer_public_keys.push(*issuer_pk);
            }
        }

        // Query metadata for uncached tokens
        let queried_metadata = self
            .query_tokens_metadata(&uncached_identifiers, &uncached_issuer_public_keys)
            .await?;

        // Combine cached and queried metadata
        let mut all_metadata = cached_metadata;
        all_metadata.extend(queried_metadata);

        Ok(all_metadata)
    }

    /// Returns the metadata for the given token identifiers as the SE reports it, without reading
    /// the local cache.
    pub async fn query_tokens_metadata(
        &self,
        token_identifiers: &[&str],
        issuer_public_keys: &[PublicKey],
    ) -> Result<Vec<TokenMetadata>, ServiceError> {
        if token_identifiers.is_empty() && issuer_public_keys.is_empty() {
            return Ok(Vec::new());
        }

        let token_identifiers = token_identifiers
            .iter()
            .map(|id| {
                bech32m_decode_token_id(id, Some(self.network))
                    .map_err(|_| ServiceError::Generic("Invalid token id".to_string()))
            })
            .collect::<Result<Vec<Vec<u8>>, _>>()?;
        let issuer_public_keys = issuer_public_keys
            .iter()
            .map(|k| k.serialize().to_vec())
            .collect::<Vec<_>>();
        let metadata = self
            .query_tokens_metadata_inner(token_identifiers, issuer_public_keys)
            .await?;
        let metadata = metadata
            .into_iter()
            .map(|m| (m, self.network).try_into())
            .collect::<Result<Vec<_>, _>>()?;
        Ok(metadata)
    }

    async fn query_tokens_metadata_inner(
        &self,
        token_identifiers: Vec<Vec<u8>>,
        issuer_public_keys: Vec<Vec<u8>>,
    ) -> Result<Vec<rpc::spark_token::TokenMetadata>, ServiceError> {
        let metadata = self
            .operator_pool
            .get_coordinator()
            .client
            .query_token_metadata(QueryTokenMetadataRequest {
                token_identifiers,
                issuer_public_keys,
            })
            .await?
            .token_metadata;
        Ok(metadata)
    }

    pub async fn query_token_transactions_inner(
        &self,
        filter: QueryTokenTransactionsFilter,
        paging: PagingFilter,
    ) -> Result<PagingResult<TokenTransaction>, ServiceError> {
        let owner_public_keys = match filter.owner_public_keys {
            Some(keys) => keys
                .iter()
                .map(|k| k.serialize().to_vec())
                .collect::<Vec<_>>(),
            None => vec![
                self.spark_signer
                    .get_identity_public_key()
                    .await?
                    .serialize()
                    .to_vec(),
            ],
        };

        let order: crate::operator::rpc::spark::Order = paging.order.into();
        let response = self
            .operator_pool
            .get_coordinator()
            .client
            .query_token_transactions(QueryTokenTransactionsRequest {
                query_type: Some(QueryType::ByFilters(QueryTokenTransactionsByFilters {
                    output_ids: filter.output_ids,
                    owner_public_keys,
                    issuer_public_keys: filter
                        .issuer_public_keys
                        .iter()
                        .map(|k| k.serialize().to_vec())
                        .collect(),
                    token_identifiers: filter
                        .token_ids
                        .iter()
                        .map(|id| {
                            bech32m_decode_token_id(id, Some(self.network))
                                .map_err(|_| ServiceError::Generic("Invalid token id".to_string()))
                        })
                        .collect::<Result<Vec<Vec<u8>>, _>>()?,
                    page_request: None,
                })),
                order: order.into(),
                limit: paging.limit as i64,
                offset: paging.offset as i64,
                ..Default::default()
            })
            .await?;

        let records: Vec<_> = response
            .token_transactions_with_status
            .into_iter()
            .map(|t| (t, self.network))
            .collect();

        Ok(PagingResult {
            items: convert_page(records, "token transaction")?,
            next: paging.next_from_offset(response.offset),
        })
    }

    pub async fn query_token_transactions(
        &self,
        filter: QueryTokenTransactionsFilter,
        paging: Option<PagingFilter>,
    ) -> Result<PagingResult<TokenTransaction>, ServiceError> {
        let transactions = match paging {
            Some(paging) => self.query_token_transactions_inner(filter, paging).await?,
            None => {
                pager(
                    |p| self.query_token_transactions_inner(filter.clone(), p),
                    PagingFilter::default(),
                )
                .await?
            }
        };
        Ok(transactions)
    }

    /// Queries token transactions by their hashes.
    ///
    /// This method uses the `QueryType::ByTxHash` variant which is limited to 100 hashes
    /// per request (enforced by proto). Returns an empty vector if input is empty.
    pub async fn query_token_transactions_by_hashes(
        &self,
        token_transaction_hashes: Vec<String>,
    ) -> Result<Vec<TokenTransaction>, ServiceError> {
        if token_transaction_hashes.is_empty() {
            return Ok(Vec::new());
        }

        let hashes = token_transaction_hashes
            .iter()
            .map(|h| {
                hex::decode(h).map_err(|_| {
                    ServiceError::Generic("Invalid token transaction hash".to_string())
                })
            })
            .collect::<Result<Vec<Vec<u8>>, _>>()?;

        let response = self
            .operator_pool
            .get_coordinator()
            .client
            .query_token_transactions(QueryTokenTransactionsRequest {
                query_type: Some(QueryType::ByTxHash(QueryTokenTransactionsByTxHash {
                    token_transaction_hashes: hashes,
                })),
                ..Default::default()
            })
            .await?;

        let records: Vec<_> = response
            .token_transactions_with_status
            .into_iter()
            .map(|t| (t, self.network))
            .collect();
        convert_page(records, "token transaction")
    }

    pub async fn get_issuer_token_metadata(&self) -> Result<TokenMetadata, ServiceError> {
        let identity_public_key = self.spark_signer.get_identity_public_key().await?;
        let tokens_metadata = self
            .get_tokens_metadata(&[], &[identity_public_key])
            .await?;
        tokens_metadata
            .into_iter()
            .find(|m| m.is_issued_by(&identity_public_key, self.network))
            .ok_or(ServiceError::Generic("No issuer token found".to_string()))
    }
}

pub fn bech32m_encode_token_id(
    raw_token_id: &[u8],
    network: Network,
) -> Result<String, ServiceError> {
    let hrp_str = match network {
        Network::Mainnet => HRP_STR_MAINNET,
        Network::Testnet => HRP_STR_TESTNET,
        Network::Regtest => HRP_STR_REGTEST,
        Network::Signet => HRP_STR_SIGNET,
    };
    let hrp = Hrp::parse_unchecked(hrp_str);
    let bech32 = bech32::encode::<Bech32m>(hrp, raw_token_id)
        .map_err(|e| ServiceError::Generic(format!("Failed to encode token id: {e}")))?;
    Ok(bech32)
}

/// Decodes a token id from a string.
///
/// If a network is provided, it will be checked against the network in the token id.
pub fn bech32m_decode_token_id(
    token_id: &str,
    network: Option<Network>,
) -> Result<Vec<u8>, ServiceError> {
    let (hrp, data) = bech32::decode(token_id)
        .map_err(|e| ServiceError::Generic(format!("Failed to decode token id: {e}")))?;
    let bech32_network = match hrp.as_str() {
        "btkn" => Network::Mainnet,
        "btknt" => Network::Testnet,
        "btknrt" => Network::Regtest,
        "btkns" => Network::Signet,
        _ => return Err(ServiceError::Generic(format!("Invalid network: {hrp}"))),
    };
    if let Some(network) = network
        && bech32_network != network
    {
        return Err(ServiceError::Generic(format!(
            "Invalid network: {bech32_network}"
        )));
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use macros::test_all;

    use crate::{Network, operator::rpc};

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    #[test_all]
    fn test_bech32m_decode_token_id() {
        let token_id = "btkn1xgrvjwey5ngcagvap2dzzvsy4uk8ua9x69k82dwvt5e7ef9drm9qztux87";
        let decoded = super::bech32m_decode_token_id(token_id, Some(Network::Mainnet)).unwrap();
        assert_eq!(
            hex::encode(&decoded),
            "3206c93b24a4d18ea19d0a9a213204af2c7e74a6d16c7535cc5d33eca4ad1eca"
        );
    }

    #[test_all]
    fn test_bech32m_encode_token_id() {
        let raw_token_id =
            hex::decode("ee2f1dc42cf0866420f2b0195bb3607199a730c85d7138214b6ad09b55e47542")
                .unwrap();
        let encoded = super::bech32m_encode_token_id(&raw_token_id, Network::Regtest).unwrap();
        assert_eq!(
            encoded,
            "btknrt1ach3m3pv7zrxgg8jkqv4hvmqwxv6wvxgt4cnsg2tdtgfk40yw4pq98h0dl"
        );

        let decoded = super::bech32m_decode_token_id(&encoded, Some(Network::Regtest)).unwrap();
        assert_eq!(decoded, raw_token_id);
    }

    #[test_all]
    fn test_final_token_output_conversion_populates_payment_required_fields() {
        use crate::token::TokenOutput;

        // secp256k1 generator G — on-curve, so PublicKey::from_slice accepts it.
        let owner_pubkey =
            hex::decode("0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798")
                .unwrap();
        let token_id = vec![0xab; 32];
        let amount_bytes = 1000u128.to_be_bytes().to_vec();
        let revocation_commitment = vec![0xcd; 33];

        let final_output = rpc::spark_token::FinalTokenOutput {
            partial_token_output: Some(rpc::spark_token::PartialTokenOutput {
                owner_public_key: owner_pubkey.clone(),
                withdraw_bond_sats: 10_000,
                withdraw_relative_block_locktime: 1_000,
                token_identifier: token_id.clone(),
                token_amount: amount_bytes,
            }),
            revocation_commitment: revocation_commitment.clone(),
        };

        let local: TokenOutput = (final_output, Network::Regtest)
            .try_into()
            .expect("FinalTokenOutput must convert to local TokenOutput");

        assert_eq!(local.owner_public_key.serialize().to_vec(), owner_pubkey);
        assert_eq!(local.token_amount, 1000);
        assert!(
            local.token_identifier.starts_with("btknrt1"),
            "expected regtest btknrt1 prefix, got {}",
            local.token_identifier
        );
        assert_eq!(local.withdraw_bond_sats, 10_000);
        assert_eq!(local.withdraw_relative_block_locktime, 1_000);
        assert_eq!(
            local.revocation_commitment,
            hex::encode(&revocation_commitment)
        );
        assert!(local.token_public_key.is_none());
    }
}
