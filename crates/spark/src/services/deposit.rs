use std::{collections::HashSet, str::FromStr, sync::Arc};

use bitcoin::{
    Address, Amount, OutPoint, Transaction, TxOut, Txid, Witness,
    address::NetworkUnchecked,
    consensus::serialize,
    hashes::{Hash, sha256},
    params::Params,
    secp256k1::{Message, PublicKey, ecdsa::Signature, schnorr},
};
use serde::Serialize;
use tracing::{error, trace};

use crate::{
    Network,
    bitcoin::{BitcoinService, sighash_from_tx, verify_finalized_taproot_signature},
    operator::{
        OperatorPool,
        rpc::{self as operator_rpc, spark::HashVariant},
    },
    services::{SigningResult, Utxo, models::map_signing_nonce_commitments},
    signer::{
        FrostDerivation, FrostJob, PrepareStaticDepositClaimRequest, PreparedStaticDepositClaim,
        SignStaticDepositRefundRequest, SparkSigner, StartStaticDepositRefundRequest,
        StartedStaticDepositRefund,
    },
    ssp::{ClaimStaticDepositInput, ClaimStaticDepositRequestType, ServiceProvider},
    tree::{TreeNode, TreeNodeId},
    utils::{
        paging::{PagingFilter, PagingResult, pager},
        tagged_hasher::TaggedHasher,
        transactions::{
            NodeTransactions, RefundTransactions, create_initial_timelock_refund_txs,
            create_root_node_txs, create_static_deposit_refund_tx,
        },
    },
};

use super::ServiceError;

const CLAIM_STATIC_DEPOSIT_ACTION: &str = "claim_static_deposit";

// Conservative minimum fee threshold for refund transactions
// Based on 194 vbyte estimate for 1-in/1-out tx at 1 sat/vB minimum relay fee.
const MIN_REFUND_FEE_SATS: u64 = 194;

/// Witness vbytes for a single Schnorr signature: ceil(66 witness bytes / 4)
/// Witness structure: 1 (stack items) + 1 (sig length varint) + 64 (signature) = 66 bytes
const SCHNORR_SIG_WITNESS_VBYTES: u64 = 17;

/// A static deposit address.
#[derive(Debug)]
pub struct StaticDepositAddress {
    pub address: Address,
    pub user_signing_public_key: PublicKey,
    pub verifying_public_key: PublicKey,
}

/// A non-static deposit address that includes a leaf ID for tree creation.
#[derive(Debug)]
pub struct SingleUseDepositAddress {
    pub address: Address,
    pub user_signing_public_key: PublicKey,
    pub verifying_public_key: PublicKey,
    pub leaf_id: TreeNodeId,
}

#[derive(Debug, Copy, Clone)]
pub enum Fee {
    Fixed { amount: u64 },
    Rate { sat_per_vbyte: u64 },
}

impl Fee {
    pub fn to_sats(&self, vbytes: u64) -> u64 {
        match self {
            Fee::Fixed { amount } => *amount,
            Fee::Rate { sat_per_vbyte } => sat_per_vbyte * vbytes,
        }
    }
}

fn parse_deposit_address_result(
    result: &operator_rpc::spark::DepositAddressQueryResult,
    network: Network,
) -> Result<(Address, PublicKey, PublicKey), ServiceError> {
    let address: Address<NetworkUnchecked> = result
        .deposit_address
        .parse()
        .map_err(|_| ServiceError::InvalidDepositAddress)?;
    let address = address
        .require_network(network.into())
        .map_err(|_| ServiceError::InvalidDepositAddressNetwork)?;
    let user_signing_public_key = PublicKey::from_slice(&result.user_signing_public_key)
        .map_err(|_| ServiceError::InvalidDepositAddressProof)?;
    let verifying_public_key = PublicKey::from_slice(&result.verifying_public_key)
        .map_err(|_| ServiceError::InvalidDepositAddressProof)?;
    Ok((address, user_signing_public_key, verifying_public_key))
}

impl TryFrom<(operator_rpc::spark::DepositAddressQueryResult, Network)> for StaticDepositAddress {
    type Error = ServiceError;

    fn try_from(
        (result, network): (operator_rpc::spark::DepositAddressQueryResult, Network),
    ) -> Result<Self, Self::Error> {
        let (address, user_signing_public_key, verifying_public_key) =
            parse_deposit_address_result(&result, network)?;
        Ok(StaticDepositAddress {
            address,
            user_signing_public_key,
            verifying_public_key,
        })
    }
}

impl TryFrom<(operator_rpc::spark::DepositAddressQueryResult, Network)>
    for SingleUseDepositAddress
{
    type Error = ServiceError;

    fn try_from(
        (result, network): (operator_rpc::spark::DepositAddressQueryResult, Network),
    ) -> Result<Self, Self::Error> {
        let leaf_id: TreeNodeId = result
            .leaf_id
            .as_ref()
            .ok_or(ServiceError::MissingLeafId)?
            .parse()
            .map_err(ServiceError::InvalidNodeId)?;
        let (address, user_signing_public_key, verifying_public_key) =
            parse_deposit_address_result(&result, network)?;
        Ok(SingleUseDepositAddress {
            address,
            user_signing_public_key,
            verifying_public_key,
            leaf_id,
        })
    }
}

#[derive(Debug, Serialize)]
pub struct StaticDepositQuote {
    pub txid: Txid,
    pub output_index: u32,
    pub credit_amount_sats: u64,
    pub signature: Signature,
}

impl TryFrom<crate::ssp::StaticDepositQuote> for StaticDepositQuote {
    type Error = ServiceError;

    fn try_from(quote: crate::ssp::StaticDepositQuote) -> Result<Self, Self::Error> {
        let txid =
            Txid::from_str(&quote.transaction_id).map_err(|_| ServiceError::InvalidTransaction)?;
        let signature = Signature::from_str(&quote.signature)
            .map_err(|_| ServiceError::InvalidSignatureShare)?;
        Ok(StaticDepositQuote {
            txid,
            output_index: quote.output_index as u32,
            credit_amount_sats: quote.credit_amount_sats,
            signature,
        })
    }
}

#[derive(Clone, Copy, Debug)]
#[repr(u8)]
pub enum UtxoSwapRequestType {
    Fixed,
    MaxFee,
    Refund,
}

pub struct DepositService {
    bitcoin_service: BitcoinService,
    identity_public_key: PublicKey,
    network: Network,
    operator_pool: Arc<OperatorPool>,
    ssp_client: Arc<ServiceProvider>,
    spark_signer: Arc<dyn SparkSigner>,
}

impl DepositService {
    pub fn new(
        bitcoin_service: BitcoinService,
        identity_public_key: PublicKey,
        network: impl Into<Network>,
        operator_pool: Arc<OperatorPool>,
        ssp_client: Arc<ServiceProvider>,
        spark_signer: Arc<dyn SparkSigner>,
    ) -> Self {
        DepositService {
            bitcoin_service,
            identity_public_key,
            network: network.into(),
            operator_pool,
            ssp_client,
            spark_signer,
        }
    }

    pub async fn get_utxos_for_identity(
        &self,
        page_size: u32,
        cursor: Option<String>,
    ) -> Result<(Vec<Utxo>, Option<String>), ServiceError> {
        let res = self
            .operator_pool
            .get_coordinator()
            .client
            .get_utxos_for_identity(operator_rpc::spark::GetUtxosForIdentityRequest {
                identity_public_key: self.identity_public_key.serialize().to_vec(),
                network: self.network.to_proto_network() as i32,
                exclude_claimed: true,
                page: Some(operator_rpc::spark::PageRequest {
                    page_size,
                    cursor: cursor.unwrap_or_default(),
                    ..Default::default()
                }),
                include_pending: true,
            })
            .await?;
        let utxos = res
            .utxos
            .into_iter()
            .map(|au| {
                au.utxo.ok_or(ServiceError::MissingUtxo).and_then(|u| {
                    Utxo::from_proto(u, au.is_confirmed /* proto field maps to is_mature */)
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let next_cursor = res.page.filter(|p| p.has_next_page).map(|p| p.next_cursor);
        Ok((utxos, next_cursor))
    }

    pub async fn claim_deposit(
        &self,
        deposit_tx: Transaction,
        vout: u32,
    ) -> Result<Vec<TreeNode>, ServiceError> {
        // TODO: Ensure all inputs are segwit inputs, so this tx is not malleable. Normally the tx should be already confirmed, but perhaps we get in trouble with a reorg?
        let params: Params = self.network.into();

        let output: &TxOut = deposit_tx
            .output
            .get(vout as usize)
            .ok_or(ServiceError::InvalidOutputIndex)?;
        let address = Address::from_script(&output.script_pubkey, params)
            .map_err(|_| ServiceError::NotADepositOutput)?;
        let deposit_address = self
            .get_unused_deposit_address(&address)
            .await?
            .ok_or(ServiceError::DepositAddressUsed)?;
        let deposit_leaf_id = deposit_address.leaf_id;
        self.create_tree_root(
            &deposit_leaf_id,
            &deposit_address.verifying_public_key,
            deposit_tx,
            vout,
        )
        .await
    }

    /// Submits a static deposit claim to the SSP and returns the resulting
    /// transfer id. The transfer can then be looked up by id via the
    /// transfer service / `SparkWallet::list_transfers`.
    pub async fn claim_static_deposit(
        &self,
        quote: StaticDepositQuote,
    ) -> Result<String, ServiceError> {
        trace!("Claiming static deposit with quote: {quote:?}");
        let StaticDepositQuote {
            txid,
            output_index,
            credit_amount_sats,
            signature: quote_signature,
        } = quote;

        // Serialize the static deposit claim user-statement.
        let user_statement = self.serialize_static_deposit_claim_payload(
            txid,
            output_index,
            UtxoSwapRequestType::Fixed,
            credit_amount_sats,
            &quote_signature.serialize_der(),
        );

        // The signer exports the static-deposit secret and signs the
        // user-statement with the identity key.
        let PreparedStaticDepositClaim {
            deposit_secret_key,
            user_signature,
        } = self
            .spark_signer
            .prepare_static_deposit_claim(PrepareStaticDepositClaimRequest {
                index: 0,
                user_statement,
            })
            .await?;

        // The SSP co-signs the claim and so needs the static-deposit secret.
        // Send it ECIES-encrypted to the SSP identity public key (same scheme as
        // transfer leaf secret ciphers) instead of in cleartext over GraphQL.
        let encrypted_deposit_secret_key = utils::ecies::encrypt(
            &self
                .ssp_client
                .identity_public_key()
                .serialize_uncompressed(),
            &deposit_secret_key.secret_bytes(),
        )
        .map_err(|e| {
            ServiceError::Generic(format!("ECIES encryption of deposit key failed: {e}"))
        })?;

        // Call the service provider to claim the static deposit
        let resp = self
            .ssp_client
            .claim_static_deposit(ClaimStaticDepositInput {
                transaction_id: txid.to_string(),
                output_index: output_index as i64,
                network: self.network.into(),
                credit_amount_sats: Some(credit_amount_sats),
                request_type: Some(ClaimStaticDepositRequestType::FixedAmount),
                max_fee_sats: None,
                deposit_secret_key: None,
                encrypted_deposit_secret_key: Some(hex::encode(encrypted_deposit_secret_key)),
                quote_signature: quote_signature.serialize_der().to_string(),
                signature: user_signature.serialize_der().to_string(),
            })
            .await?;

        Ok(resp.transfer_id)
    }

    pub async fn refund_static_deposit(
        &self,
        tx: Transaction,
        output_index: Option<u32>,
        refund_address: Address,
        fee: Fee,
    ) -> Result<Transaction, ServiceError> {
        let txid = tx.compute_txid();
        let output_index = match output_index {
            Some(v) => v,
            None => self
                .find_static_deposit_tx_vout(&tx)
                .await?
                .ok_or(ServiceError::InvalidOutputIndex)?,
        };
        let tx_out = tx
            .output
            .get(output_index as usize)
            .ok_or(ServiceError::InvalidOutputIndex)?;

        // Create the refund transaction with a dummy output amount.
        // The witness vsize is accounted for separately via SCHNORR_SIG_WITNESS_VBYTES.
        let mut refund_tx = create_static_deposit_refund_tx(
            OutPoint {
                txid,
                vout: output_index,
            },
            0, // Temporary value for calculating the vsize. We set the real value below.
            &refund_address,
        );

        // Account for witness data that will be added after signing
        let signed_vsize = refund_tx.vsize() as u64 + SCHNORR_SIG_WITNESS_VBYTES;
        let fee_sats = fee.to_sats(signed_vsize);
        if fee_sats < MIN_REFUND_FEE_SATS {
            return Err(ServiceError::Generic(format!(
                "fee must be at least {} sats",
                MIN_REFUND_FEE_SATS
            )));
        }

        let credit_amount_sats = tx_out.value.to_sat().saturating_sub(fee_sats);
        refund_tx.output[0].value = Amount::from_sat(credit_amount_sats);

        // Validate the output amount meets the dust limit for this address type
        let dust_limit = refund_address.script_pubkey().minimal_non_dust();
        if Amount::from_sat(credit_amount_sats) < dust_limit {
            return Err(ServiceError::InvalidInput(format!(
                "Refund amount ({credit_amount_sats} sats) is below the minimum of {} sats required for this address",
                dust_limit.to_sat()
            )));
        }
        trace!(
            "Refunding static deposit txid: {txid}, output_index: {output_index}, credit_amount_sats: {credit_amount_sats}, fee_sats: {fee_sats}"
        );

        let spend_tx_sighash = sighash_from_tx(&refund_tx, 0, tx_out)?;

        // Serialize the static deposit refund user-statement.
        let user_statement = self.serialize_static_deposit_claim_payload(
            txid,
            output_index,
            UtxoSwapRequestType::Refund,
            credit_amount_sats,
            spend_tx_sighash.as_byte_array(),
        );

        // Begin the refund (user-commits-first): the signer returns the
        // static-deposit signing key, a user nonce commitment to forward to the
        // operators, and the identity-key signature over the user-statement.
        let StartedStaticDepositRefund {
            signing_public_key,
            nonce_commitment,
            user_signature,
        } = self
            .spark_signer
            .start_static_deposit_refund(StartStaticDepositRefundRequest {
                index: 0,
                user_statement,
            })
            .await?;

        let refund_resp = self
            .operator_pool
            .get_coordinator()
            .client
            .initiate_static_deposit_utxo_refund(
                operator_rpc::spark::InitiateStaticDepositUtxoRefundRequest {
                    on_chain_utxo: Some(operator_rpc::spark::Utxo {
                        vout: output_index,
                        network: self.network.to_proto_network() as i32,
                        txid: hex::decode(txid.to_string())
                            .map_err(|_| ServiceError::InvalidTransaction)?,
                        ..Default::default()
                    }),
                    user_signature: user_signature.serialize_der().to_vec(),
                    refund_tx_signing_job: Some(operator_rpc::spark::SigningJob {
                        signing_public_key: signing_public_key.serialize().to_vec(),
                        raw_tx: serialize(&refund_tx),
                        signing_nonce_commitment: Some(nonce_commitment.commitments.try_into()?),
                    }),
                    hash_variant: 0,
                },
            )
            .await?;

        // Collect and map the signing results
        let signing_result: SigningResult = refund_resp
            .refund_tx_signing_result
            .as_ref()
            .map(|sr| sr.try_into())
            .transpose()?
            .ok_or(ServiceError::MissingTreeSignatures)?;

        let verifying_public_key = refund_resp
            .deposit_address
            .map(|da| PublicKey::from_slice(&da.verifying_public_key))
            .transpose()
            .map_err(|_| ServiceError::InvalidPublicKey)?
            .ok_or(ServiceError::InvalidVerifyingKey)?;

        // Finish the refund: the signer produces the user's FROST share (bound
        // to the committed nonce) and aggregates it with the operators' shares.
        let spend_signature = self
            .spark_signer
            .sign_static_deposit_refund(SignStaticDepositRefundRequest {
                index: 0,
                sighash: *spend_tx_sighash.as_byte_array(),
                verifying_key: verifying_public_key,
                nonce_commitment,
                statechain_commitments: signing_result.signing_commitments,
                statechain_signatures: signing_result.signature_shares,
                statechain_public_keys: signing_result.public_keys,
            })
            .await?;

        // Update the input with the aggregated signature
        let mut witness = Witness::new();
        witness.push(&spend_signature.serialize()?);
        refund_tx.input[0].witness = witness;

        Ok(refund_tx)
    }

    fn serialize_static_deposit_claim_payload(
        &self,
        txid: Txid,
        output_index: u32,
        request_type: UtxoSwapRequestType,
        credit_amount_sats: u64,
        signing_payload: &[u8],
    ) -> Vec<u8> {
        // The user statement is constructed by concatenating the following fields in order:
        // 1. Action name: "claim_static_deposit" (UTF-8 string)
        let mut payload = CLAIM_STATIC_DEPOSIT_ACTION.as_bytes().to_vec();
        // 2. Network: lowercase network name (e.g., "bitcoin", "testnet") (UTF-8 string)
        payload.extend_from_slice(self.network.to_string().as_bytes());
        // 3. Transaction ID: hex-encoded UTXO transaction ID (UTF-8 string)
        payload.extend_from_slice(txid.to_string().as_bytes());
        // 4. Output index: UTXO output index (vout) as 4-byte unsigned integer (little-endian)
        payload.extend_from_slice(&output_index.to_le_bytes());
        // 5. Request type (1-byte unsigned integer, little-endian)
        payload.extend_from_slice(&[request_type as u8]);
        // 6. Credit amount: amount of satoshis to credit as 8-byte unsigned integer (little-endian)
        payload.extend_from_slice(&credit_amount_sats.to_le_bytes());
        // 7. Signing payload: SSP signature or sighash of spend transaction (UTF-8 string)
        payload.extend_from_slice(signing_payload);
        payload
    }

    /// Creates a tree root node for a deposit transaction.
    ///
    /// This function initializes the transaction structure for a new deposit in the Spark protocol.
    /// It creates multiple transactions to ensure security and flexibility in fund management:
    ///
    /// Transaction Structure:
    /// ```ignore
    ///                           +---------------+
    ///                           | Deposit TX    |
    ///                           | (On-chain)    |
    ///                           +-------+-------+
    ///                                   |
    ///                     +-------------+--------------+
    ///                     |                            |
    ///           +---------v----------+       +---------v----------+
    ///           | CPFP Root TX       |       | Direct Root TX     |
    ///           | (anchor, no fee)   |       | (no anchor, fee)   |
    ///           +---------+----------+       +---------+----------+
    ///                     |                            |
    ///      +--------------+-------------+              +----------+
    ///      |                            |                         |
    /// +----v-------------+      +-------v----------+       +------v-----------+
    /// | CPFP Refund TX   |      | Direct From CPFP |       | Direct Refund TX |
    /// | (anchor, no fee) |      | Refund TX        |       | (no anchor, fee) |
    /// |                  |      | (no anchor, fee) |       |                  |
    /// +------------------+      +------------------+       +------------------+
    /// ```
    ///
    /// The function:
    /// 1. Creates a pair of root transactions (CPFP and Direct) that spend from the deposit
    /// 2. Creates three refund transactions to ensure funds can be recovered:
    ///    - CPFP Refund TX: Spends from CPFP Root TX, includes anchor output for fee bumping
    ///    - Direct Refund TX: Spends from Direct Root TX, no anchor output
    ///    - Direct-from-CPFP Refund TX: Alternative path that spends from CPFP Root TX using direct sequence
    /// 3. Sets up signing commitments for all transactions
    /// 4. Signs all transactions using FROST threshold signatures
    /// 5. Finalizes and registers the node with operators
    ///
    /// # Arguments
    ///
    /// * `deposit_leaf_id` - The ID for the leaf node being created
    /// * `verifying_public_key` - The public key used to verify signatures
    /// * `deposit_tx` - The on-chain deposit transaction
    /// * `vout` - The output index in the deposit transaction
    ///
    /// # Returns
    ///
    /// * `Ok(Vec<TreeNode>)` - The created tree nodes
    /// * `Err(ServiceError)` - If any part of the creation process fails
    async fn create_tree_root(
        &self,
        deposit_leaf_id: &TreeNodeId,
        verifying_public_key: &PublicKey,
        deposit_tx: Transaction,
        vout: u32,
    ) -> Result<Vec<TreeNode>, ServiceError> {
        let signing_public_key = self
            .spark_signer
            .get_public_key_for_leaf(deposit_leaf_id)
            .await?;

        let deposit_txid = deposit_tx.compute_txid();
        let deposit_tx_out = deposit_tx
            .output
            .get(vout as usize)
            .ok_or(ServiceError::InvalidOutputIndex)?;

        let NodeTransactions {
            cpfp_tx: cpfp_root_tx,
            direct_tx: _,
        } = create_root_node_txs(&deposit_tx, vout)?;

        let RefundTransactions {
            cpfp_tx: cpfp_refund_tx,
            direct_tx: _,
            direct_from_cpfp_tx: direct_from_cpfp_refund_tx,
        } = create_initial_timelock_refund_txs(
            &cpfp_root_tx,
            None,
            &signing_public_key,
            self.network,
        );

        let Some(direct_from_cpfp_refund_tx) = direct_from_cpfp_refund_tx else {
            return Err(ServiceError::Generic(
                "Direct from CPFP refund transaction is missing".to_string(),
            ));
        };

        // Fetch operator signing commitments. The tree node does not exist yet,
        // so request commitments by count rather than by node id.
        let signing_commitments = self
            .operator_pool
            .get_coordinator()
            .client
            .get_signing_commitments(operator_rpc::spark::GetSigningCommitmentsRequest {
                node_ids: Vec::new(),
                count: 3,
                node_id_count: 1,
            })
            .await?
            .signing_commitments;

        let [
            cpfp_root_commitments,
            cpfp_refund_commitments,
            direct_from_cpfp_refund_commitments,
        ] = signing_commitments.as_slice()
        else {
            return Err(ServiceError::Generic(format!(
                "Expected 3 signing commitments, got {}",
                signing_commitments.len()
            )));
        };

        // Compute sighashes for all transactions
        let cpfp_root_sighash = sighash_from_tx(&cpfp_root_tx, 0, deposit_tx_out)?;
        let cpfp_refund_sighash = sighash_from_tx(&cpfp_refund_tx, 0, &cpfp_root_tx.output[0])?;
        let direct_from_cpfp_refund_sighash =
            sighash_from_tx(&direct_from_cpfp_refund_tx, 0, &cpfp_root_tx.output[0])?;

        // The user produces a FROST signature share for each transaction; the
        // operators aggregate server-side during finalization. The deposit
        // tree-root signing key is the deposit leaf's signing key.
        let derivation = FrostDerivation::SigningLeaf {
            leaf_id: deposit_leaf_id.clone(),
        };
        let jobs = vec![
            FrostJob {
                derivation: derivation.clone(),
                sighash: *cpfp_root_sighash.as_byte_array(),
                verifying_key: *verifying_public_key,
                operator_commitments: map_signing_nonce_commitments(
                    &cpfp_root_commitments.signing_nonce_commitments,
                )?,
                adaptor_public_key: None,
            },
            FrostJob {
                derivation: derivation.clone(),
                sighash: *cpfp_refund_sighash.as_byte_array(),
                verifying_key: *verifying_public_key,
                operator_commitments: map_signing_nonce_commitments(
                    &cpfp_refund_commitments.signing_nonce_commitments,
                )?,
                adaptor_public_key: None,
            },
            FrostJob {
                derivation,
                sighash: *direct_from_cpfp_refund_sighash.as_byte_array(),
                verifying_key: *verifying_public_key,
                operator_commitments: map_signing_nonce_commitments(
                    &direct_from_cpfp_refund_commitments.signing_nonce_commitments,
                )?,
                adaptor_public_key: None,
            },
        ];
        let [
            cpfp_root_share,
            cpfp_refund_share,
            direct_from_cpfp_refund_share,
        ] = self
            .spark_signer
            .sign_frost(jobs)
            .await?
            .try_into()
            .map_err(|v: Vec<_>| {
                ServiceError::Generic(format!("Expected 3 FROST shares, got {}", v.len()))
            })?;

        let finalize_resp = self
            .operator_pool
            .get_coordinator()
            .client
            .finalize_deposit_tree_creation(
                operator_rpc::spark::FinalizeDepositTreeCreationRequest {
                    identity_public_key: self.identity_public_key.serialize().to_vec(),
                    on_chain_utxo: Some(operator_rpc::spark::Utxo {
                        raw_tx: serialize(&deposit_tx),
                        vout,
                        network: self.network.to_proto_network() as i32,
                        txid: deposit_txid.as_byte_array().to_vec(),
                    }),
                    root_tx_signing_job: Some(operator_rpc::spark::UserSignedTxSigningJob {
                        leaf_id: String::new(),
                        signing_public_key: signing_public_key.serialize().to_vec(),
                        raw_tx: serialize(&cpfp_root_tx),
                        signing_nonce_commitment: Some(
                            cpfp_root_share.commitment.commitments.try_into()?,
                        ),
                        user_signature: cpfp_root_share.signature_share.serialize().to_vec(),
                        signing_commitments: Some(operator_rpc::spark::SigningCommitments {
                            signing_commitments: cpfp_root_commitments
                                .signing_nonce_commitments
                                .clone(),
                        }),
                        additional_inputs: Vec::new(),
                    }),
                    refund_tx_signing_job: Some(operator_rpc::spark::UserSignedTxSigningJob {
                        leaf_id: String::new(),
                        signing_public_key: signing_public_key.serialize().to_vec(),
                        raw_tx: serialize(&cpfp_refund_tx),
                        signing_nonce_commitment: Some(
                            cpfp_refund_share.commitment.commitments.try_into()?,
                        ),
                        user_signature: cpfp_refund_share.signature_share.serialize().to_vec(),
                        signing_commitments: Some(operator_rpc::spark::SigningCommitments {
                            signing_commitments: cpfp_refund_commitments
                                .signing_nonce_commitments
                                .clone(),
                        }),
                        additional_inputs: Vec::new(),
                    }),
                    direct_from_cpfp_refund_tx_signing_job: Some(
                        operator_rpc::spark::UserSignedTxSigningJob {
                            leaf_id: String::new(),
                            signing_public_key: signing_public_key.serialize().to_vec(),
                            raw_tx: serialize(&direct_from_cpfp_refund_tx),
                            signing_nonce_commitment: Some(
                                direct_from_cpfp_refund_share
                                    .commitment
                                    .commitments
                                    .try_into()?,
                            ),
                            user_signature: direct_from_cpfp_refund_share
                                .signature_share
                                .serialize()
                                .to_vec(),
                            signing_commitments: Some(operator_rpc::spark::SigningCommitments {
                                signing_commitments: direct_from_cpfp_refund_commitments
                                    .signing_nonce_commitments
                                    .clone(),
                            }),
                            additional_inputs: Vec::new(),
                        },
                    ),
                    additional_on_chain_utxos: Vec::new(),
                },
            )
            .await?;

        let root_node = finalize_resp.root_node.ok_or_else(|| {
            ServiceError::Generic(
                "finalize_deposit_tree_creation returned no root node".to_string(),
            )
        })?;

        // Verify the operator-returned root node matches what we signed for.
        // The fused `start_deposit_tree_creation` flow returned signature shares
        // that we aggregated locally; the package flow aggregates server-side,
        // so we re-derive the same security guarantees here:
        //  1) the verifying key we used really is the tree's verifying key,
        //  2) each returned transaction carries a valid Schnorr signature
        //     under that key for the sighashes we computed.
        let returned_verifying_key = PublicKey::from_slice(&root_node.verifying_public_key)
            .map_err(|_| ServiceError::InvalidVerifyingKey)?;
        if &returned_verifying_key != verifying_public_key {
            return Err(ServiceError::InvalidVerifyingKey);
        }
        verify_finalized_taproot_signature(
            &self.bitcoin_service,
            &root_node.node_tx,
            cpfp_root_sighash.as_byte_array(),
            verifying_public_key,
        )?;
        verify_finalized_taproot_signature(
            &self.bitcoin_service,
            &root_node.refund_tx,
            cpfp_refund_sighash.as_byte_array(),
            verifying_public_key,
        )?;
        verify_finalized_taproot_signature(
            &self.bitcoin_service,
            &root_node.direct_from_cpfp_refund_tx,
            direct_from_cpfp_refund_sighash.as_byte_array(),
            verifying_public_key,
        )?;

        Ok(vec![root_node.try_into()?])
    }

    pub async fn generate_deposit_address(
        &self,
        signing_public_key: PublicKey,
        leaf_id: &TreeNodeId,
    ) -> Result<SingleUseDepositAddress, ServiceError> {
        let resp = self
            .operator_pool
            .get_coordinator()
            .client
            .generate_deposit_address(operator_rpc::spark::GenerateDepositAddressRequest {
                signing_public_key: signing_public_key.serialize().to_vec(),
                identity_public_key: self.identity_public_key.serialize().to_vec(),
                network: self.network.to_proto_network() as i32,
                leaf_id: Some(leaf_id.to_string()),
                is_static: None,
                hash_variant: HashVariant::V2.into(),
            })
            .await?;

        let Some(deposit_address) = resp.deposit_address else {
            return Err(ServiceError::MissingDepositAddress);
        };

        self.validate_deposit_address(deposit_address, signing_public_key, leaf_id)
    }

    pub async fn generate_static_deposit_address(
        &self,
        signing_public_key: PublicKey,
    ) -> Result<StaticDepositAddress, ServiceError> {
        let resp = self
            .operator_pool
            .get_coordinator()
            .client
            .generate_static_deposit_address(
                operator_rpc::spark::GenerateStaticDepositAddressRequest {
                    signing_public_key: signing_public_key.serialize().to_vec(),
                    identity_public_key: self.identity_public_key.serialize().to_vec(),
                    network: self.network.to_proto_network() as i32,
                    hash_variant: HashVariant::V2.into(),
                },
            )
            .await?;

        let Some(deposit_address) = resp.deposit_address else {
            return Err(ServiceError::MissingDepositAddress);
        };

        self.validate_static_deposit_address(deposit_address, signing_public_key)
    }

    pub async fn rotate_static_deposit_address(
        &self,
        signing_public_key: PublicKey,
    ) -> Result<StaticDepositAddress, ServiceError> {
        let resp = self
            .operator_pool
            .get_coordinator()
            .client
            .rotate_static_deposit_address(operator_rpc::spark::RotateStaticDepositAddressRequest {
                signing_public_key: signing_public_key.serialize().to_vec(),
                network: self.network.to_proto_network() as i32,
                hash_variant: HashVariant::V2.into(),
            })
            .await?;

        let new_deposit_address = resp
            .new_deposit_address
            .ok_or(ServiceError::MissingDepositAddress)?;

        self.validate_static_deposit_address(new_deposit_address, signing_public_key)
    }

    async fn query_static_deposit_addresses_inner(
        &self,
        paging: PagingFilter,
    ) -> Result<PagingResult<StaticDepositAddress>, ServiceError> {
        trace!(
            "Querying static deposit addresses with limit: {:?}, offset: {:?}",
            paging.limit, paging.offset
        );
        let resp = self
            .operator_pool
            .get_coordinator()
            .client
            .query_static_deposit_addresses(
                operator_rpc::spark::QueryStaticDepositAddressesRequest {
                    identity_public_key: self.identity_public_key.serialize().to_vec(),
                    network: self.network.to_proto_network() as i32,
                    offset: paging.offset as i64,
                    limit: paging.limit as i64,
                    ..Default::default()
                },
            )
            .await?;

        let addresses = resp
            .deposit_addresses
            .into_iter()
            .map(|result| (result, self.network).try_into())
            .collect::<Result<Vec<_>, ServiceError>>()
            .map_err(|_| ServiceError::InvalidDepositAddress)?;

        // There is no offset in the static addresses response
        Ok(PagingResult::complete(addresses))
    }

    pub async fn query_static_deposit_addresses(
        &self,
        paging: Option<PagingFilter>,
    ) -> Result<PagingResult<StaticDepositAddress>, ServiceError> {
        let result = match paging {
            Some(paging) => self.query_static_deposit_addresses_inner(paging).await?,
            None => {
                pager(
                    |f| self.query_static_deposit_addresses_inner(f),
                    PagingFilter::default(),
                )
                .await?
            }
        };
        Ok(result)
    }

    pub async fn get_unused_deposit_address(
        &self,
        address: &Address,
    ) -> Result<Option<SingleUseDepositAddress>, ServiceError> {
        // TODO: unused deposit addresses could be cached in the wallet, so they don't have to be queried from the server every time.
        let addresses = self.query_unused_deposit_addresses(None).await?;
        Ok(addresses.items.into_iter().find(|d| &d.address == address))
    }

    async fn query_unused_deposit_addresses_inner(
        &self,
        paging: PagingFilter,
    ) -> Result<PagingResult<SingleUseDepositAddress>, ServiceError> {
        trace!(
            "Querying unused deposit addresses with limit: {:?}, offset: {:?}",
            paging.limit, paging.offset
        );
        let resp = self
            .operator_pool
            .get_coordinator()
            .client
            .query_unused_deposit_addresses(
                operator_rpc::spark::QueryUnusedDepositAddressesRequest {
                    identity_public_key: self.identity_public_key.serialize().to_vec(),
                    network: self.network.to_proto_network() as i32,
                    offset: paging.offset as i64,
                    limit: paging.limit as i64,
                },
            )
            .await?;

        let addresses = resp
            .deposit_addresses
            .into_iter()
            .filter_map(
                |result| match SingleUseDepositAddress::try_from((result, self.network)) {
                    Ok(addr) => Some(addr),
                    Err(ServiceError::MissingLeafId) => {
                        error!("Ignoring deposit address without leaf ID");
                        None
                    }
                    Err(e) => {
                        error!("Failed to parse deposit address: {e}");
                        None
                    }
                },
            )
            .collect();

        Ok(PagingResult {
            items: addresses,
            next: paging.next_from_offset(resp.offset),
        })
    }

    pub async fn query_unused_deposit_addresses(
        &self,
        paging: Option<PagingFilter>,
    ) -> Result<PagingResult<SingleUseDepositAddress>, ServiceError> {
        let addresses = match paging {
            Some(paging) => self.query_unused_deposit_addresses_inner(paging).await?,
            None => {
                pager(
                    |f| self.query_unused_deposit_addresses_inner(f),
                    PagingFilter::default(),
                )
                .await?
            }
        };
        Ok(addresses)
    }

    pub async fn fetch_static_deposit_claim_quote(
        &self,
        tx: Transaction,
        output_index: Option<u32>,
    ) -> Result<StaticDepositQuote, ServiceError> {
        let output_index = match output_index {
            Some(v) => v,
            None => self
                .find_static_deposit_tx_vout(&tx)
                .await?
                .ok_or(ServiceError::InvalidOutputIndex)?,
        };
        let static_deposit_quote = self
            .ssp_client
            .get_claim_deposit_quote(
                tx.compute_txid().to_string(),
                output_index,
                self.network.into(),
            )
            .await?;

        static_deposit_quote.try_into()
    }

    async fn find_static_deposit_tx_vout(
        &self,
        tx: &Transaction,
    ) -> Result<Option<u32>, ServiceError> {
        let static_addresses: HashSet<Address> = self
            .query_static_deposit_addresses(None)
            .await?
            .items
            .into_iter()
            .map(|a| a.address)
            .collect();
        let params: Params = self.network.into();

        for (vout, tx_out) in tx.output.iter().enumerate() {
            if let Ok(address) = Address::from_script(&tx_out.script_pubkey, &params) {
                // Check if the address is a static deposit address
                if static_addresses.contains(&address) {
                    return Ok(Some(vout as u32));
                }
            }
        }

        Ok(None)
    }

    fn proof_of_possession_message_hash(
        &self,
        operator_public_key: &PublicKey,
        address: &Address,
    ) -> sha256::Hash {
        TaggedHasher::new(&["spark", "deposit", "proof_of_possession"])
            .add_bytes(&self.identity_public_key.serialize())
            .add_bytes(&operator_public_key.serialize())
            .add_bytes(address.to_string().as_bytes())
            .hash()
    }

    fn validate_deposit_address(
        &self,
        deposit_address: crate::operator::rpc::spark::Address,
        user_signing_public_key: PublicKey,
        leaf_id: &TreeNodeId,
    ) -> Result<SingleUseDepositAddress, ServiceError> {
        let (address, verifying_public_key) =
            self.validate_deposit_address_inner(deposit_address, user_signing_public_key, false)?;

        Ok(SingleUseDepositAddress {
            address,
            user_signing_public_key,
            verifying_public_key,
            leaf_id: leaf_id.clone(),
        })
    }

    fn validate_static_deposit_address(
        &self,
        deposit_address: crate::operator::rpc::spark::Address,
        user_signing_public_key: PublicKey,
    ) -> Result<StaticDepositAddress, ServiceError> {
        let (address, verifying_public_key) =
            self.validate_deposit_address_inner(deposit_address, user_signing_public_key, true)?;

        Ok(StaticDepositAddress {
            address,
            user_signing_public_key,
            verifying_public_key,
        })
    }

    fn validate_deposit_address_inner(
        &self,
        deposit_address: crate::operator::rpc::spark::Address,
        user_signing_public_key: PublicKey,
        verify_coordinator_proof: bool,
    ) -> Result<(Address, PublicKey), ServiceError> {
        let address: Address<NetworkUnchecked> = deposit_address
            .address
            .parse()
            .map_err(|_| ServiceError::InvalidDepositAddress)?;
        let address = address
            .require_network(self.network.into())
            .map_err(|_| ServiceError::InvalidDepositAddressNetwork)?;

        let Some(proof) = deposit_address.deposit_address_proof else {
            return Err(ServiceError::MissingDepositAddressProof);
        };

        let verifying_public_key = PublicKey::from_slice(&deposit_address.verifying_key)
            .map_err(|_| ServiceError::InvalidDepositAddressProof)?;

        let operator_public_key = self
            .bitcoin_service
            .subtract_public_keys(&verifying_public_key, &user_signing_public_key)
            .map_err(|_| ServiceError::InvalidDepositAddressProof)?;
        let taproot_key = self
            .bitcoin_service
            .compute_taproot_key_no_script(&operator_public_key);

        // Note this is not a proof of possession really, but rather a commitment by the server that they associate the address with the user's identity.
        let msg = self.proof_of_possession_message_hash(&operator_public_key, &address);
        let msg = Message::from_digest(msg.to_byte_array());
        let proof_of_possession_signature =
            schnorr::Signature::from_slice(&proof.proof_of_possession_signature)
                .map_err(|_| ServiceError::InvalidDepositAddressProof)?;
        if !self.bitcoin_service.is_valid_schnorr_signature(
            &proof_of_possession_signature,
            &msg,
            &taproot_key,
        ) {
            error!(
                "Deposit address {} has invalid proof of possession signature for operator {}",
                address, operator_public_key
            );
            return Err(ServiceError::InvalidDepositAddressProof);
        }

        let coordinator_identifier = self.operator_pool.get_coordinator().identifier;
        let address_hash = sha256::Hash::hash(address.to_string().as_bytes());
        let address_hash_message = Message::from_digest(address_hash.to_byte_array());
        for operator in self.operator_pool.get_all_operators() {
            if operator.identifier == coordinator_identifier && !verify_coordinator_proof {
                continue;
            }
            // TODO: rather than using hex::encode here, we should define our own type for the frost identifier, and use a hashmap with the identifier as key here.
            let Some(operator_sig) = proof
                .address_signatures
                .get(&hex::encode(operator.identifier.serialize()))
            else {
                error!(
                    "Deposit address {} misses signature for operator {}",
                    address, operator.id
                );
                return Err(ServiceError::InvalidDepositAddressProof);
            };

            let Ok(operator_sig) = Signature::from_der(operator_sig) else {
                error!(
                    "Failed to parse ECDSA signature for operator {}",
                    operator.id
                );
                return Err(ServiceError::InvalidDepositAddressProof);
            };

            if !self.bitcoin_service.is_valid_ecdsa_signature(
                &operator_sig,
                &address_hash_message,
                &operator.identity_public_key,
            ) {
                error!(
                    "Deposit address {} has invalid signature for operator {}",
                    address, operator.id
                );
                return Err(ServiceError::InvalidDepositAddressProof);
            }
        }

        Ok((address, verifying_public_key))
    }
}
