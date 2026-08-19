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
    ssp::{
        ClaimStaticDepositInput, ClaimStaticDepositRequestType,
        CreateClaimInstantStaticDepositInput, InstantStaticDepositPlan, InstantStaticDepositQuote,
        InstantStaticDepositQuoteResult, ServiceProvider,
    },
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

// Domain tag and request-type discriminant for the instant (0-conf) claim user
// statement, which is the BIP-340 tagged-hash pre-image (tag_hash || tag_hash ||
// fields) the signer sha256-hashes, not a finished hash. Request type 3 is Instant.
const CLAIM_INSTANT_STATIC_DEPOSIT_TAG: [&str; 2] = ["spark", "claim_instant_static_deposit"];
const INSTANT_UTXO_SWAP_REQUEST_TYPE: u64 = 3;

/// Every static deposit address of a wallet shares one signing key: rotation
/// changes the operators' share, not the user's.
const STATIC_DEPOSIT_KEY_INDEX: u32 = 0;

/// Bitcoin Core's default minimum relay fee rate. The refund fee floor is this
/// rate applied to the real signed size of the refund transaction, which varies
/// with the refund address type.
const MIN_RELAY_FEE_SAT_PER_VBYTE: u64 = 1;

/// Witness vbytes for a single Schnorr signature: ceil(66 witness bytes / 4)
/// Witness structure: 1 (stack items) + 1 (sig length varint) + 64 (signature) = 66 bytes
const SCHNORR_SIG_WITNESS_VBYTES: u64 = 17;

/// Builds the instant static deposit claim user statement. The field order and
/// encoding must byte-match what the SSP validates for a 0-conf claim.
fn serialize_instant_static_deposit_claim_payload(
    network: &str,
    credit_amount_sats: u64,
    deposit_amount_sats: u64,
    static_deposit_address: &str,
    quote_signature: &[u8],
) -> Vec<u8> {
    TaggedHasher::new(&CLAIM_INSTANT_STATIC_DEPOSIT_TAG)
        .add_string(network)
        .add_u64(INSTANT_UTXO_SWAP_REQUEST_TYPE)
        .add_u64(credit_amount_sats)
        .add_u64(0) // secondary credit amount, always 0
        .add_string(static_deposit_address)
        .add_u64(deposit_amount_sats)
        .add_bytes(quote_signature)
        .signable_message()
}

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

/// Checks that funds sent to `address` are spendable by the FROST aggregate
/// key, which is what makes the address the user's rather than the operators'.
/// The expected output is the BIP-341 key-path P2TR over the verifying key with
/// an empty script tree, the same key spends are verified against. Operators
/// choose both the address and the verifying key, so nothing else ties the two.
fn validate_address_pays_to_verifying_key(
    bitcoin_service: &BitcoinService,
    address: &Address,
    verifying_public_key: &PublicKey,
) -> Result<(), ServiceError> {
    let expected = bitcoin_service.p2tr_address(verifying_public_key.x_only_public_key().0, None);
    if address.script_pubkey() != expected.script_pubkey() {
        error!(
            "Deposit address {address} does not pay to verifying key {verifying_public_key}, expected {expected}"
        );
        return Err(ServiceError::DepositAddressKeyMismatch);
    }

    Ok(())
}

/// The same check as [`validate_address_pays_to_verifying_key`], against the
/// output a spend is being signed for rather than a listed address. Without it
/// a wrong verifying key only surfaces as a refund transaction that fails to
/// broadcast, since the aggregate signature is then over a key the output does
/// not commit to.
fn validate_output_pays_to_verifying_key(
    bitcoin_service: &BitcoinService,
    tx_out: &TxOut,
    verifying_public_key: &PublicKey,
) -> Result<(), ServiceError> {
    let expected = bitcoin_service.p2tr_address(verifying_public_key.x_only_public_key().0, None);
    if tx_out.script_pubkey != expected.script_pubkey() {
        error!(
            "Deposit output does not pay to verifying key {verifying_public_key}, expected {expected}"
        );
        return Err(ServiceError::DepositAddressKeyMismatch);
    }

    Ok(())
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

        // Sign the user-statement and export the ECIES-encrypted deposit secret
        // the SSP needs to co-sign the claim.
        let (encrypted_deposit_secret_key, user_signature) = self
            .prepare_encrypted_static_deposit_claim(user_statement)
            .await?;

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
                encrypted_deposit_secret_key: Some(encrypted_deposit_secret_key),
                quote_signature: quote_signature.serialize_der().to_string(),
                signature: user_signature.serialize_der().to_string(),
            })
            .await?;

        Ok(resp.transfer_id)
    }

    /// Signs the claim `user_statement` with the identity key and exports the
    /// static-deposit secret the SSP needs to co-sign the claim, ECIES-encrypted
    /// (hex) to the SSP identity public key (same scheme as transfer leaf secret
    /// ciphers) rather than sent in cleartext over GraphQL. Returns
    /// `(encrypted_secret_hex, user_signature)`.
    async fn prepare_encrypted_static_deposit_claim(
        &self,
        user_statement: Vec<u8>,
    ) -> Result<(String, Signature), ServiceError> {
        let PreparedStaticDepositClaim {
            deposit_secret_key,
            user_signature,
        } = self
            .spark_signer
            .prepare_static_deposit_claim(PrepareStaticDepositClaimRequest {
                index: STATIC_DEPOSIT_KEY_INDEX,
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

        Ok((hex::encode(encrypted_deposit_secret_key), user_signature))
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
        let min_fee_sats = signed_vsize * MIN_RELAY_FEE_SAT_PER_VBYTE;
        if fee_sats < min_fee_sats {
            return Err(ServiceError::Generic(format!(
                "fee must be at least {min_fee_sats} sats ({MIN_RELAY_FEE_SAT_PER_VBYTE} sat/vB over {signed_vsize} vbytes)"
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
                index: STATIC_DEPOSIT_KEY_INDEX,
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

        // The operators pick the key the refund is signed under, so tie it to
        // the output being spent before handing it to the signer.
        validate_output_pays_to_verifying_key(
            &self.bitcoin_service,
            tx_out,
            &verifying_public_key,
        )?;

        // Finish the refund: the signer produces the user's FROST share (bound
        // to the committed nonce) and aggregates it with the operators' shares.
        let spend_signature = self
            .spark_signer
            .sign_static_deposit_refund(SignStaticDepositRefundRequest {
                index: STATIC_DEPOSIT_KEY_INDEX,
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
        //  2) the returned cpfp transactions carry a valid Schnorr signature
        //     under that key for the sighashes we computed.
        // `direct_from_cpfp_refund_tx` carries no signature to verify.
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
    ) -> Result<StaticDepositAddress, ServiceError> {
        let signing_public_key = self.static_deposit_public_key().await?;
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
    ) -> Result<StaticDepositAddress, ServiceError> {
        let signing_public_key = self.static_deposit_public_key().await?;
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
        user_signing_public_key: PublicKey,
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
                    // Without this the operators hash the proof of possession
                    // message the legacy way, which the wallet cannot verify.
                    hash_variant: HashVariant::V2.into(),
                    ..Default::default()
                },
            )
            .await?;

        // An entry the wallet cannot verify is dropped rather than fatal: the
        // listing feeds claims for every other address, and the operators can
        // always serve one entry this wallet rejects, whether by bug or design.
        let addresses = resp
            .deposit_addresses
            .into_iter()
            .filter_map(|result| {
                match self.static_deposit_address_from_result(&result, user_signing_public_key) {
                    Ok(address) => Some(address),
                    Err(e) => {
                        error!("Ignoring static deposit address: {e}");
                        None
                    }
                }
            })
            .collect();

        // There is no offset in the static addresses response
        Ok(PagingResult::complete(addresses))
    }

    pub async fn query_static_deposit_addresses(
        &self,
        paging: Option<PagingFilter>,
    ) -> Result<PagingResult<StaticDepositAddress>, ServiceError> {
        // Fetched once for the whole listing: with a remote signer, doing it per
        // page would be a round trip each time.
        let user_signing_public_key = self.static_deposit_public_key().await?;
        let result = match paging {
            Some(paging) => {
                self.query_static_deposit_addresses_inner(paging, user_signing_public_key)
                    .await?
            }
            None => {
                pager(
                    |f| self.query_static_deposit_addresses_inner(f, user_signing_public_key),
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
                |result| match self.single_use_deposit_address_from_result(&result) {
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

    /// Fetch an instant static deposit quote and its fulfillment plans.
    pub async fn fetch_instant_static_deposit_quote(
        &self,
        tx: Transaction,
        output_index: Option<u32>,
    ) -> Result<InstantStaticDepositQuoteResult, ServiceError> {
        let output_index = match output_index {
            Some(v) => v,
            None => self
                .find_static_deposit_tx_vout(&tx)
                .await?
                .ok_or(ServiceError::InvalidOutputIndex)?,
        };
        let result = self
            .ssp_client
            .get_instant_static_deposit_quote(
                tx.compute_txid().to_string(),
                output_index,
                self.network.into(),
            )
            .await?;
        // Guard against the SSP quoting a different output than the one requested.
        if result.quote.output_index != i64::from(output_index) {
            return Err(ServiceError::Generic(format!(
                "instant quote output index {} does not match requested {output_index}",
                result.quote.output_index
            )));
        }
        Ok(result)
    }

    /// Claim a static deposit ahead of maturity. `tx` is the funding
    /// transaction: its output at the quote's `output_index` names the static
    /// deposit address the UTXO paid to, which the 0-conf user statement must
    /// reference. `plan` selects which statement form the SSP verifies against.
    pub async fn claim_instant_static_deposit(
        &self,
        tx: Transaction,
        quote: InstantStaticDepositQuote,
        plan: InstantStaticDepositPlan,
    ) -> Result<String, ServiceError> {
        // `tx` must be the transaction the quote was issued for: the statement is
        // signed against `tx`'s output, so a mismatched pair would sign the wrong
        // address. (Public API; in-tree the caller always pairs them correctly.)
        let quote_txid = Txid::from_str(&quote.transaction_id)
            .map_err(|e| ServiceError::Generic(format!("invalid quote transaction id: {e}")))?;
        if tx.compute_txid() != quote_txid {
            return Err(ServiceError::Generic(
                "funding tx does not match the quote transaction_id".to_string(),
            ));
        }

        // Raw bytes of the SSP quote signature (hex) go into the user statement.
        let quote_signature_bytes = hex::decode(&quote.quote_signature)
            .map_err(|e| ServiceError::Generic(format!("invalid quote signature hex: {e}")))?;

        // The SSP verifies each depth against a different statement: the tagged
        // instant hash at 0-conf, and the same statement as a normal mature claim
        // from 1-conf on. Both commit to the quote's credit amount.
        let credit_amount_sats = quote.credit_amount.original_value;
        let user_statement = if plan.confirmations == 0 {
            // The statement names the address the UTXO actually paid to, derived
            // from the funding output. The wallet's current static address may have
            // rotated since the deposit landed, so it cannot be regenerated here.
            let output_index = quote.output_index as usize;
            let tx_out = tx.output.get(output_index).ok_or_else(|| {
                ServiceError::Generic(format!("quote output_index {output_index} out of range"))
            })?;
            let params: Params = self.network.into();
            let static_deposit_address = Address::from_script(&tx_out.script_pubkey, &params)
                .map_err(|e| ServiceError::Generic(format!("invalid static deposit script: {e}")))?
                .to_string();

            serialize_instant_static_deposit_claim_payload(
                &self.network.to_string(),
                credit_amount_sats,
                quote.deposit_amount.original_value,
                &static_deposit_address,
                &quote_signature_bytes,
            )
        } else {
            let output_index = u32::try_from(quote.output_index).map_err(|_| {
                ServiceError::Generic(format!("invalid quote output_index {}", quote.output_index))
            })?;
            self.serialize_static_deposit_claim_payload(
                quote_txid,
                output_index,
                UtxoSwapRequestType::Fixed,
                credit_amount_sats,
                &quote_signature_bytes,
            )
        };

        // Sign the user-statement and export the ECIES-encrypted deposit secret
        // the SSP needs to co-sign the claim.
        let (encrypted_deposit_secret_key, user_signature) = self
            .prepare_encrypted_static_deposit_claim(user_statement)
            .await?;

        let resp = self
            .ssp_client
            .claim_instant_static_deposit(CreateClaimInstantStaticDepositInput {
                static_deposit_quote_id: quote.id,
                static_deposit_address_private_key_share: None,
                encrypted_static_deposit_address_private_key_share: Some(
                    encrypted_deposit_secret_key,
                ),
                signature: user_signature.serialize_der().to_string(),
            })
            .await?;

        Ok(resp.claim_id)
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

    async fn static_deposit_public_key(&self) -> Result<PublicKey, ServiceError> {
        Ok(self
            .spark_signer
            .get_static_deposit_public_key(STATIC_DEPOSIT_KEY_INDEX)
            .await?)
    }

    /// Validates a static deposit address the operators list back, against the
    /// caller's own signing key rather than the one in the response.
    fn static_deposit_address_from_result(
        &self,
        result: &operator_rpc::spark::DepositAddressQueryResult,
        user_signing_public_key: PublicKey,
    ) -> Result<StaticDepositAddress, ServiceError> {
        // The proof of possession is checked over the caller's key regardless,
        // so this only turns "the proof does not verify" into an error naming
        // the key the operators actually used.
        let listed_signing_public_key = PublicKey::from_slice(&result.user_signing_public_key)
            .map_err(|_| ServiceError::InvalidDepositAddressProof)?;
        if listed_signing_public_key != user_signing_public_key {
            error!(
                "Deposit address {} is listed for signing key {listed_signing_public_key}, not the wallet's {user_signing_public_key}",
                result.deposit_address
            );
            return Err(ServiceError::DepositAddressUserKeyMismatch);
        }

        let (address, verifying_public_key) = self.validate_deposit_address_inner(
            &result.deposit_address,
            &result.verifying_public_key,
            result.proof_of_possession.as_ref(),
            user_signing_public_key,
            true,
        )?;

        Ok(StaticDepositAddress {
            address,
            user_signing_public_key,
            verifying_public_key,
        })
    }

    /// These results carry no proof of possession, so only the address to
    /// verifying key binding is checked. That the key holds the user's share is
    /// established when the address is generated, not here.
    fn single_use_deposit_address_from_result(
        &self,
        result: &operator_rpc::spark::DepositAddressQueryResult,
    ) -> Result<SingleUseDepositAddress, ServiceError> {
        let leaf_id: TreeNodeId = result
            .leaf_id
            .as_ref()
            .ok_or(ServiceError::MissingLeafId)?
            .parse()
            .map_err(ServiceError::InvalidNodeId)?;

        let user_signing_public_key = PublicKey::from_slice(&result.user_signing_public_key)
            .map_err(|_| ServiceError::InvalidDepositAddressProof)?;
        let (address, verifying_public_key) = self
            .parse_bound_deposit_address(&result.deposit_address, &result.verifying_public_key)?;

        Ok(SingleUseDepositAddress {
            address,
            user_signing_public_key,
            verifying_public_key,
            leaf_id,
        })
    }

    /// Parses an operator-supplied address and its verifying key, rejecting the
    /// pair unless the address pays to that key.
    fn parse_bound_deposit_address(
        &self,
        deposit_address: &str,
        verifying_key: &[u8],
    ) -> Result<(Address, PublicKey), ServiceError> {
        let address: Address<NetworkUnchecked> = deposit_address
            .parse()
            .map_err(|_| ServiceError::InvalidDepositAddress)?;
        let address = address
            .require_network(self.network.into())
            .map_err(|_| ServiceError::InvalidDepositAddressNetwork)?;
        let verifying_public_key = PublicKey::from_slice(verifying_key)
            .map_err(|_| ServiceError::InvalidDepositAddressProof)?;

        validate_address_pays_to_verifying_key(
            &self.bitcoin_service,
            &address,
            &verifying_public_key,
        )?;

        Ok((address, verifying_public_key))
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
        let (address, verifying_public_key) = self.validate_deposit_address_inner(
            &deposit_address.address,
            &deposit_address.verifying_key,
            deposit_address.deposit_address_proof.as_ref(),
            user_signing_public_key,
            false,
        )?;

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
        let (address, verifying_public_key) = self.validate_deposit_address_inner(
            &deposit_address.address,
            &deposit_address.verifying_key,
            deposit_address.deposit_address_proof.as_ref(),
            user_signing_public_key,
            true,
        )?;

        Ok(StaticDepositAddress {
            address,
            user_signing_public_key,
            verifying_public_key,
        })
    }

    /// `user_signing_public_key` has to be the caller's own key. The proof of
    /// possession is checked against `verifying_key - user_signing_public_key`,
    /// so passing back a key the operators supplied would let them satisfy the
    /// proof with a pair they generated themselves.
    fn validate_deposit_address_inner(
        &self,
        deposit_address: &str,
        verifying_key: &[u8],
        proof: Option<&operator_rpc::spark::DepositAddressProof>,
        user_signing_public_key: PublicKey,
        verify_coordinator_proof: bool,
    ) -> Result<(Address, PublicKey), ServiceError> {
        let Some(proof) = proof else {
            return Err(ServiceError::MissingDepositAddressProof);
        };

        let (address, verifying_public_key) =
            self.parse_bound_deposit_address(deposit_address, verifying_key)?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::key::CompressedPublicKey;
    use bitcoin::secp256k1::{Secp256k1, SecretKey};
    use macros::test_all;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    const NETWORK: bitcoin::Network = bitcoin::Network::Regtest;

    fn test_key(seed: u8) -> PublicKey {
        let secp = Secp256k1::new();
        let secret = SecretKey::from_slice(&[seed; 32]).unwrap();
        PublicKey::from_secret_key(&secp, &secret)
    }

    fn p2tr(key: &PublicKey) -> Address {
        let secp = Secp256k1::new();
        Address::p2tr(&secp, key.x_only_public_key().0, None, NETWORK)
    }

    #[test_all]
    fn accepts_p2tr_over_the_verifying_key() {
        let service = BitcoinService::new(NETWORK);
        let verifying_key = test_key(1);

        assert!(
            validate_address_pays_to_verifying_key(&service, &p2tr(&verifying_key), &verifying_key)
                .is_ok()
        );
    }

    #[test_all]
    fn rejects_p2tr_over_another_key() {
        let service = BitcoinService::new(NETWORK);
        let operator_key = test_key(2);

        assert!(matches!(
            validate_address_pays_to_verifying_key(&service, &p2tr(&operator_key), &test_key(1)),
            Err(ServiceError::DepositAddressKeyMismatch)
        ));
    }

    /// The untweaked key spends nothing on its own: only the BIP-341 tweaked
    /// output key does, so an address over the raw key must not pass.
    #[test_all]
    fn rejects_untweaked_key_output() {
        let service = BitcoinService::new(NETWORK);
        let verifying_key = test_key(1);
        let untweaked = Address::p2tr_tweaked(
            bitcoin::key::TweakedPublicKey::dangerous_assume_tweaked(
                verifying_key.x_only_public_key().0,
            ),
            NETWORK,
        );

        assert!(matches!(
            validate_address_pays_to_verifying_key(&service, &untweaked, &verifying_key),
            Err(ServiceError::DepositAddressKeyMismatch)
        ));
    }

    #[test_all]
    fn rejects_non_taproot_output_for_the_verifying_key() {
        let service = BitcoinService::new(NETWORK);
        let verifying_key = test_key(1);
        let p2wpkh = Address::p2wpkh(&CompressedPublicKey(verifying_key), NETWORK);

        assert!(matches!(
            validate_address_pays_to_verifying_key(&service, &p2wpkh, &verifying_key),
            Err(ServiceError::DepositAddressKeyMismatch)
        ));
    }

    fn tx_out(address: &Address) -> TxOut {
        TxOut {
            value: Amount::from_sat(10_000),
            script_pubkey: address.script_pubkey(),
        }
    }

    #[test_all]
    fn accepts_output_paying_to_the_verifying_key() {
        let service = BitcoinService::new(NETWORK);
        let verifying_key = test_key(1);

        assert!(
            validate_output_pays_to_verifying_key(
                &service,
                &tx_out(&p2tr(&verifying_key)),
                &verifying_key
            )
            .is_ok()
        );
    }

    #[test_all]
    fn rejects_output_paying_to_another_key() {
        let service = BitcoinService::new(NETWORK);
        let operator_key = test_key(2);

        assert!(matches!(
            validate_output_pays_to_verifying_key(
                &service,
                &tx_out(&p2tr(&operator_key)),
                &test_key(1)
            ),
            Err(ServiceError::DepositAddressKeyMismatch)
        ));
    }

    /// The fee floor is `MIN_RELAY_FEE_SAT_PER_VBYTE * signed_vsize`, so the
    /// unsigned vsize plus `SCHNORR_SIG_WITNESS_VBYTES` must equal the vsize the
    /// transaction actually has once the key-path signature is attached.
    #[test_all]
    fn signed_vsize_estimate_matches_the_signed_transaction() {
        let outpoint = OutPoint {
            txid: Txid::from_slice(&[7u8; 32]).unwrap(),
            vout: 0,
        };
        let key = test_key(1);
        let refund_addresses = [
            p2tr(&key),
            Address::p2wpkh(&CompressedPublicKey(key), NETWORK),
            Address::p2pkh(CompressedPublicKey(key), NETWORK),
        ];

        for address in refund_addresses {
            let mut tx = create_static_deposit_refund_tx(outpoint, 0, &address);
            let estimated = tx.vsize() as u64 + SCHNORR_SIG_WITNESS_VBYTES;

            tx.input[0].witness = Witness::from_slice(&[[0u8; 64]]);

            assert_eq!(estimated, tx.vsize() as u64, "address: {address}");
        }
    }
}
