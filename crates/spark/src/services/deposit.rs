use std::{collections::HashSet, str::FromStr, sync::Arc};

use bitcoin::{
    Address, OutPoint, Transaction, TxOut, Txid, Witness,
    address::NetworkUnchecked,
    consensus::{deserialize, serialize},
    hashes::{Hash, sha256},
    params::Params,
    secp256k1::{Message, PublicKey, ecdsa::Signature, schnorr},
};
use serde::Serialize;
use tracing::{error, trace};

use crate::{
    Network,
    bitcoin::{BitcoinService, sighash_from_tx},
    core::initial_sequence,
    operator::{
        OperatorPool,
        rpc::{
            self as operator_rpc,
            spark::{TransferFilter, transfer_filter::Participant},
        },
    },
    services::Transfer,
    signer::{AggregateFrostRequest, PrivateKeySource, SignFrostRequest, Signer},
    ssp::{ClaimStaticDepositInput, ClaimStaticDepositRequestType, ServiceProvider},
    tree::{TreeNode, TreeNodeId},
    utils::{
        paging::{PagingFilter, PagingResult, pager},
        transactions::{create_node_tx, create_refund_tx, create_static_deposit_refund_tx},
    },
};

use super::{
    ServiceError,
    models::{map_public_keys, map_signature_shares, map_signing_nonce_commitments},
};

const CLAIM_STATIC_DEPOSIT_ACTION: &str = "claim_static_deposit";

#[derive(Debug)]
pub struct DepositAddress {
    pub address: Address,
    pub leaf_id: TreeNodeId,
    pub user_signing_public_key: PublicKey,
    pub verifying_public_key: PublicKey,
}

impl TryFrom<(operator_rpc::spark::DepositAddressQueryResult, Network)> for DepositAddress {
    type Error = ServiceError;

    fn try_from(
        (result, network): (operator_rpc::spark::DepositAddressQueryResult, Network),
    ) -> Result<Self, Self::Error> {
        let address: Address<NetworkUnchecked> = result
            .deposit_address
            .parse()
            .map_err(|_| ServiceError::InvalidDepositAddress)?;

        Ok(DepositAddress {
            address: address
                .require_network(network.into())
                .map_err(|_| ServiceError::InvalidDepositAddressNetwork)?,
            // TODO: Is it possible addresses do not have a leaf_id?
            leaf_id: result
                .leaf_id
                .ok_or(ServiceError::MissingLeafId)?
                .parse()
                .map_err(ServiceError::InvalidNodeId)?,
            user_signing_public_key: PublicKey::from_slice(&result.user_signing_public_key)
                .map_err(|_| ServiceError::InvalidDepositAddressProof)?,
            verifying_public_key: PublicKey::from_slice(&result.verifying_public_key)
                .map_err(|_| ServiceError::InvalidDepositAddressProof)?,
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

#[derive(Clone, Debug)]
#[repr(u8)]
pub enum UtxoSwapRequestType {
    Fixed,
    MaxFee,
    Refund,
}

pub struct DepositService<S> {
    bitcoin_service: BitcoinService,
    identity_public_key: PublicKey,
    network: Network,
    operator_pool: Arc<OperatorPool<S>>,
    ssp_client: Arc<ServiceProvider<S>>,
    signer: Arc<S>,
}

impl<S: Signer> DepositService<S> {
    pub fn new(
        bitcoin_service: BitcoinService,
        identity_public_key: PublicKey,
        network: impl Into<Network>,
        operator_pool: Arc<OperatorPool<S>>,
        ssp_client: Arc<ServiceProvider<S>>,
        signer: Arc<S>,
    ) -> Self {
        DepositService {
            bitcoin_service,
            identity_public_key,
            network: network.into(),
            operator_pool,
            ssp_client,
            signer,
        }
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
        let nodes = self
            .create_tree_root(
                &deposit_address.leaf_id,
                &deposit_address.verifying_public_key,
                deposit_tx,
                vout,
            )
            .await?;
        Ok(nodes)
    }

    pub async fn claim_static_deposit(
        &self,
        quote: StaticDepositQuote,
    ) -> Result<Transfer, ServiceError> {
        trace!("Claiming static deposit with quote: {quote:?}");
        let StaticDepositQuote {
            txid,
            output_index,
            credit_amount_sats,
            signature: quote_signature,
        } = quote;

        // Serialize the static deposit claim payload
        let payload = self.serialize_static_deposit_claim_payload(
            txid,
            output_index,
            UtxoSwapRequestType::Fixed,
            credit_amount_sats,
            &quote_signature.serialize_der(),
        );
        // Sign the payload with the identity key
        let signature = self.signer.sign_message_ecdsa_with_identity_key(&payload)?;

        // TODO: Seems unavoidable to use the static deposit secret key here
        let deposit_secret_key = self
            .signer
            .get_static_deposit_private_key(0)
            .map_err(ServiceError::SignerError)?;

        // Call the service provider to claim the static deposit
        let resp = self
            .ssp_client
            .claim_static_deposit(ClaimStaticDepositInput {
                transaction_id: txid.to_string(),
                output_index: output_index as i64,
                network: self.network.into(),
                credit_amount_sats: Some(credit_amount_sats),
                request_type: ClaimStaticDepositRequestType::FixedAmount,
                max_fee_sats: None,
                deposit_secret_key: hex::encode(deposit_secret_key.secret_bytes()),
                quote_signature: quote_signature.serialize_der().to_string(),
                signature: signature.serialize_der().to_string(),
            })
            .await?;

        // Fetch the transfer from the operator pool coordinator
        let transfers: operator_rpc::spark::QueryTransfersResponse = self
            .operator_pool
            .get_coordinator()
            .client
            .query_all_transfers(TransferFilter {
                participant: Some(Participant::ReceiverIdentityPublicKey(
                    self.signer.get_identity_public_key()?.serialize().to_vec(),
                )),
                transfer_ids: vec![resp.transfer_id],
                network: self.network.to_proto_network() as i32,
                ..Default::default()
            })
            .await?;
        let transfer = transfers
            .transfers
            .into_iter()
            .nth(0)
            .ok_or(ServiceError::Generic("transfer not found".to_string()))?;

        transfer.try_into()
    }

    pub async fn refund_static_deposit(
        &self,
        tx: Transaction,
        output_index: Option<u32>,
        refund_address: Address,
        fee_sats: u64,
    ) -> Result<Transaction, ServiceError> {
        // TODO: Take a fee rate as a parameter instead of a fixed fee
        if fee_sats <= 300 {
            return Err(ServiceError::Generic(
                "fee must be more than 300 sats".to_string(),
            ));
        }

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
        let credit_amount_sats = tx_out.value.to_sat().saturating_sub(fee_sats);

        if credit_amount_sats == 0 {
            return Err(ServiceError::Generic(
                "credit amount must be more than 0 sats".to_string(),
            ));
        }
        trace!(
            "Refunding static deposit txid: {txid}, output_index: {output_index}, credit_amount_sats: {credit_amount_sats}, fee_sats: {fee_sats}"
        );

        // Create the refund transaction
        let mut refund_tx = create_static_deposit_refund_tx(
            OutPoint {
                txid,
                vout: output_index,
            },
            credit_amount_sats,
            &refund_address,
        );
        let spend_tx_sighash = sighash_from_tx(&refund_tx, 0, tx_out)?;
        let spend_nonce_commitment = self.signer.generate_frost_signing_commitments().await?;

        // Serialize the static deposit claim payload
        let payload = self.serialize_static_deposit_claim_payload(
            txid,
            output_index,
            UtxoSwapRequestType::Refund,
            credit_amount_sats,
            spend_tx_sighash.as_byte_array(),
        );
        // Sign the payload with the identity key
        let signature = self.signer.sign_message_ecdsa_with_identity_key(&payload)?;

        // Create the UTXO swap request
        let utxo_swap_resp = self
            .operator_pool
            .get_coordinator()
            .client
            .initiate_utxo_swap(operator_rpc::spark::InitiateUtxoSwapRequest {
                on_chain_utxo: Some(operator_rpc::spark::Utxo {
                    vout: output_index,
                    network: self.network.to_proto_network() as i32,
                    txid: hex::decode(txid.to_string())
                        .map_err(|_| ServiceError::InvalidTransaction)?,
                    ..Default::default()
                }),
                request_type: operator_rpc::spark::UtxoSwapRequestType::Refund as i32,
                ssp_signature: Vec::new(),
                user_signature: signature.serialize_der().to_vec(),
                transfer: Some(operator_rpc::spark::StartTransferRequest {
                    transfer_id: uuid::Uuid::now_v7().to_string(),
                    owner_identity_public_key: self.identity_public_key.serialize().to_vec(),
                    receiver_identity_public_key: self.identity_public_key.serialize().to_vec(),
                    ..Default::default()
                }),
                spend_tx_signing_job: Some(operator_rpc::spark::SigningJob {
                    signing_public_key: self
                        .signer
                        .get_static_deposit_public_key(0)?
                        .serialize()
                        .to_vec(),
                    raw_tx: serialize(&refund_tx),
                    signing_nonce_commitment: Some(spend_nonce_commitment.commitments.try_into()?),
                }),
                amount: None,
            })
            .await?;

        // Collect and map the signing results
        let spend_tx_signing_result = utxo_swap_resp
            .spend_tx_signing_result
            .ok_or(ServiceError::MissingTreeSignatures)?;
        let verifying_public_key = utxo_swap_resp
            .deposit_address
            .map(|da| PublicKey::from_slice(&da.verifying_public_key))
            .transpose()
            .map_err(|_| ServiceError::InvalidPublicKey)?
            .ok_or(ServiceError::InvalidVerifyingKey)?;

        if spend_tx_signing_result.signing_nonce_commitments.is_empty() {
            return Err(ServiceError::MissingTreeSignatures);
        }

        let spend_tx_signing_nonce_commitments =
            map_signing_nonce_commitments(&spend_tx_signing_result.signing_nonce_commitments)?;
        let spend_tx_signature_shares =
            map_signature_shares(&spend_tx_signing_result.signature_shares)?;
        let spend_tx_statechain_public_keys =
            map_public_keys(&spend_tx_signing_result.public_keys)?;

        let static_deposit_private_key_source =
            self.signer.get_static_deposit_private_key_source(0)?;
        let static_deposit_public_key = self.signer.get_static_deposit_public_key(0)?;

        let spend_sig = self
            .signer
            .sign_frost(SignFrostRequest {
                message: spend_tx_sighash.as_byte_array(),
                public_key: &verifying_public_key,
                private_key: &static_deposit_private_key_source,
                verifying_key: &verifying_public_key,
                self_nonce_commitment: &spend_nonce_commitment,
                statechain_commitments: spend_tx_signing_nonce_commitments.clone(),
                adaptor_public_key: None,
            })
            .await?;

        let spend_aggregate = self
            .signer
            .aggregate_frost(AggregateFrostRequest {
                message: spend_tx_sighash.as_byte_array(),
                statechain_signatures: spend_tx_signature_shares,
                statechain_public_keys: spend_tx_statechain_public_keys,
                verifying_key: &verifying_public_key,
                statechain_commitments: spend_tx_signing_nonce_commitments,
                self_commitment: &spend_nonce_commitment.commitments,
                public_key: &static_deposit_public_key,
                self_signature: &spend_sig,
                adaptor_public_key: None,
            })
            .await?;

        // Update the input the aggregated signature
        let mut witness = Witness::new();
        witness.push(&spend_aggregate.serialize()?);
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
        payload.extend_from_slice(&[request_type.clone() as u8]);
        // 6. Credit amount: amount of satoshis to credit as 8-byte unsigned integer (little-endian)
        payload.extend_from_slice(&credit_amount_sats.to_le_bytes());
        // 7. Signing payload: SSP signature or sighash of spend transaction (UTF-8 string)
        payload.extend_from_slice(signing_payload);
        payload
    }

    async fn create_tree_root(
        &self,
        deposit_leaf_id: &TreeNodeId,
        verifying_public_key: &PublicKey,
        deposit_tx: Transaction,
        vout: u32,
    ) -> Result<Vec<TreeNode>, ServiceError> {
        let signing_private_key = PrivateKeySource::Derived(deposit_leaf_id.clone());
        let signing_public_key = self
            .signer
            .get_public_key_from_private_key_source(&signing_private_key)?;

        let deposit_txid = deposit_tx.compute_txid();
        let deposit_output = deposit_tx
            .output
            .get(vout as usize)
            .ok_or(ServiceError::InvalidOutputIndex)?;

        let root_tx = create_node_tx(
            Default::default(),
            OutPoint {
                txid: deposit_txid,
                vout,
            },
            deposit_output.value,
            deposit_output.script_pubkey.clone(),
        );

        let refund_tx = create_refund_tx(
            initial_sequence(),
            OutPoint {
                txid: root_tx.compute_txid(),
                vout: 0,
            },
            deposit_output.value.to_sat(),
            &signing_public_key,
            self.network,
        );

        // Get random signing commitments
        let root_nonce_commitment = self.signer.generate_frost_signing_commitments().await?;
        let refund_nonce_commitment = self.signer.generate_frost_signing_commitments().await?;

        let tree_resp = self
            .operator_pool
            .get_coordinator()
            .client
            .start_deposit_tree_creation(operator_rpc::spark::StartDepositTreeCreationRequest {
                identity_public_key: self.identity_public_key.serialize().to_vec(),
                on_chain_utxo: Some(operator_rpc::spark::Utxo {
                    raw_tx: serialize(&deposit_tx),
                    vout,
                    network: self.network.to_proto_network() as i32,
                    txid: deposit_txid.as_byte_array().to_vec(),
                }),
                root_tx_signing_job: Some(operator_rpc::spark::SigningJob {
                    signing_public_key: signing_public_key.serialize().to_vec(),
                    raw_tx: serialize(&root_tx),
                    signing_nonce_commitment: Some(root_nonce_commitment.commitments.try_into()?),
                }),
                refund_tx_signing_job: Some(operator_rpc::spark::SigningJob {
                    signing_public_key: signing_public_key.serialize().to_vec(),
                    raw_tx: serialize(&refund_tx),
                    signing_nonce_commitment: Some(refund_nonce_commitment.commitments.try_into()?),
                }),
            })
            .await?;

        let root_node_signature_shares = tree_resp
            .root_node_signature_shares
            .ok_or(ServiceError::MissingTreeSignatures)?;
        let node_tx_signing_result = root_node_signature_shares
            .node_tx_signing_result
            .ok_or(ServiceError::MissingTreeSignatures)?;
        let refund_tx_signing_result = root_node_signature_shares
            .refund_tx_signing_result
            .ok_or(ServiceError::MissingTreeSignatures)?;

        if node_tx_signing_result.signing_nonce_commitments.is_empty() {
            return Err(ServiceError::MissingTreeSignatures);
        }

        let node_tx_signing_nonce_commitments =
            map_signing_nonce_commitments(&node_tx_signing_result.signing_nonce_commitments)?;
        let node_tx_signature_shares =
            map_signature_shares(&node_tx_signing_result.signature_shares)?;
        let node_tx_statechain_public_keys = map_public_keys(&node_tx_signing_result.public_keys)?;

        if refund_tx_signing_result
            .signing_nonce_commitments
            .is_empty()
        {
            return Err(ServiceError::MissingTreeSignatures);
        }

        let refund_tx_signing_nonce_commitments =
            map_signing_nonce_commitments(&refund_tx_signing_result.signing_nonce_commitments)?;
        let refund_tx_signature_shares =
            map_signature_shares(&refund_tx_signing_result.signature_shares)?;
        let refund_tx_statechain_public_keys =
            map_public_keys(&refund_tx_signing_result.public_keys)?;

        let tree_resp_verifying_key =
            PublicKey::from_slice(&root_node_signature_shares.verifying_key)
                .map_err(|_| ServiceError::InvalidVerifyingKey)?;

        if &tree_resp_verifying_key != verifying_public_key {
            return Err(ServiceError::InvalidVerifyingKey);
        }

        let root_tx_sighash = sighash_from_tx(&root_tx, 0, deposit_output)?;
        let root_sig = self
            .signer
            .sign_frost(SignFrostRequest {
                message: root_tx_sighash.as_byte_array(),
                public_key: &signing_public_key,
                private_key: &signing_private_key,
                verifying_key: verifying_public_key,
                self_nonce_commitment: &root_nonce_commitment,
                statechain_commitments: node_tx_signing_nonce_commitments.clone(),
                adaptor_public_key: None,
            })
            .await?;

        let refund_tx_sighash = sighash_from_tx(&refund_tx, 0, &root_tx.output[0])?;
        let refund_sig = self
            .signer
            .sign_frost(SignFrostRequest {
                message: refund_tx_sighash.as_byte_array(),
                public_key: &signing_public_key,
                private_key: &signing_private_key,
                verifying_key: verifying_public_key,
                self_nonce_commitment: &refund_nonce_commitment,
                statechain_commitments: refund_tx_signing_nonce_commitments.clone(),
                adaptor_public_key: None,
            })
            .await?;

        let root_aggregate = self
            .signer
            .aggregate_frost(AggregateFrostRequest {
                message: root_tx_sighash.as_byte_array(),
                statechain_signatures: node_tx_signature_shares,
                statechain_public_keys: node_tx_statechain_public_keys,
                verifying_key: verifying_public_key,
                statechain_commitments: node_tx_signing_nonce_commitments,
                self_commitment: &root_nonce_commitment.commitments,
                public_key: &signing_public_key,
                self_signature: &root_sig,
                adaptor_public_key: None,
            })
            .await?;
        let refund_aggregate = self
            .signer
            .aggregate_frost(AggregateFrostRequest {
                message: refund_tx_sighash.as_byte_array(),
                statechain_signatures: refund_tx_signature_shares,
                statechain_public_keys: refund_tx_statechain_public_keys,
                verifying_key: verifying_public_key,
                statechain_commitments: refund_tx_signing_nonce_commitments,
                self_commitment: &refund_nonce_commitment.commitments,
                public_key: &signing_public_key,
                self_signature: &refund_sig,
                adaptor_public_key: None,
            })
            .await?;

        let finalize_resp = self
            .operator_pool
            .get_coordinator()
            .client
            .finalize_node_signatures(operator_rpc::spark::FinalizeNodeSignaturesRequest {
                intent: operator_rpc::common::SignatureIntent::Creation as i32,
                node_signatures: vec![operator_rpc::spark::NodeSignatures {
                    node_id: root_node_signature_shares.node_id,
                    node_tx_signature: root_aggregate
                        .serialize()
                        .map_err(|_| ServiceError::InvalidSignatureShare)?
                        .to_vec(),
                    refund_tx_signature: refund_aggregate
                        .serialize()
                        .map_err(|_| ServiceError::InvalidSignatureShare)?
                        .to_vec(),
                }],
            })
            .await?;

        // TODO: Verify the returned tx signatures

        let nodes = finalize_resp
            .nodes
            .into_iter()
            .map(|node| {
                let signing_keyshare = node
                    .signing_keyshare
                    .ok_or(ServiceError::MissingSigningKeyshare)?;

                Ok(TreeNode {
                    id: node
                        .id
                        .parse()
                        .map_err(|_| ServiceError::InvalidNodeId(node.id))?,
                    tree_id: node.tree_id,
                    value: node.value,
                    parent_node_id: match node.parent_node_id {
                        Some(id) => Some(id.parse().map_err(|_| ServiceError::InvalidNodeId(id))?),
                        None => None,
                    },
                    node_tx: deserialize(&node.node_tx)
                        .map_err(|_| ServiceError::InvalidTransaction)?,
                    refund_tx: Some(
                        deserialize(&node.refund_tx)
                            .map_err(|_| ServiceError::InvalidTransaction)?,
                    ),
                    vout: node.vout,
                    verifying_public_key: PublicKey::from_slice(&node.verifying_public_key)
                        .map_err(|_| ServiceError::InvalidVerifyingKey)?,
                    owner_identity_public_key: PublicKey::from_slice(
                        &node.owner_identity_public_key,
                    )
                    .map_err(|_| ServiceError::InvalidPublicKey)?,
                    signing_keyshare: signing_keyshare.try_into()?,
                    status: node
                        .status
                        .parse()
                        .map_err(|_| ServiceError::UnknownStatus(node.status.clone()))?,
                })
            })
            .collect::<Result<Vec<_>, ServiceError>>()?;

        Ok(nodes)
    }

    pub async fn generate_deposit_address(
        &self,
        signing_public_key: PublicKey,
        leaf_id: &TreeNodeId,
        is_static: bool,
    ) -> Result<DepositAddress, ServiceError> {
        let resp = self
            .operator_pool
            .get_coordinator()
            .client
            .generate_deposit_address(operator_rpc::spark::GenerateDepositAddressRequest {
                signing_public_key: signing_public_key.serialize().to_vec(),
                identity_public_key: self.identity_public_key.serialize().to_vec(),
                network: self.network.to_proto_network() as i32,
                leaf_id: Some(leaf_id.to_string()),
                is_static: Some(is_static),
            })
            .await?;

        let Some(deposit_address) = resp.deposit_address else {
            return Err(ServiceError::MissingDepositAddress);
        };

        let address =
            self.validate_deposit_address(deposit_address, signing_public_key, leaf_id)?;

        Ok(address)
    }

    async fn query_static_deposit_addresses_inner(
        &self,
        paging: PagingFilter,
    ) -> Result<PagingResult<DepositAddress>, ServiceError> {
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
        Ok(PagingResult {
            items: addresses,
            next: None,
        })
    }

    pub async fn query_static_deposit_addresses(
        &self,
        paging: Option<PagingFilter>,
    ) -> Result<Vec<DepositAddress>, ServiceError> {
        let addresses = match paging {
            Some(paging) => {
                self.query_static_deposit_addresses_inner(paging)
                    .await?
                    .items
            }
            None => {
                pager(
                    |f| self.query_static_deposit_addresses_inner(f),
                    PagingFilter::default(),
                )
                .await?
            }
        };
        Ok(addresses)
    }

    pub async fn get_unused_deposit_address(
        &self,
        address: &Address,
    ) -> Result<Option<DepositAddress>, ServiceError> {
        // TODO: unused deposit addresses could be cached in the wallet, so they don't have to be queried from the server every time.
        let addresses = self.query_unused_deposit_addresses(None).await?;
        Ok(addresses.into_iter().find(|d| &d.address == address))
    }

    async fn query_unused_deposit_addresses_inner(
        &self,
        paging: PagingFilter,
    ) -> Result<PagingResult<DepositAddress>, ServiceError> {
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
            .map(|result| (result, self.network).try_into())
            .collect::<Result<Vec<_>, ServiceError>>()
            .map_err(|_| ServiceError::InvalidDepositAddress)?;

        Ok(PagingResult {
            items: addresses,
            next: paging.next_from_offset(resp.offset),
        })
    }

    pub async fn query_unused_deposit_addresses(
        &self,
        paging: Option<PagingFilter>,
    ) -> Result<Vec<DepositAddress>, ServiceError> {
        let addresses = match paging {
            Some(paging) => {
                self.query_unused_deposit_addresses_inner(paging)
                    .await?
                    .items
            }
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
        let mut msg = self.identity_public_key.serialize().to_vec();
        msg.extend_from_slice(&operator_public_key.serialize());
        msg.extend_from_slice(address.to_string().as_bytes());
        sha256::Hash::hash(&msg)
    }

    fn validate_deposit_address(
        &self,
        deposit_address: crate::operator::rpc::spark::Address,
        user_signing_public_key: PublicKey,
        leaf_id: &TreeNodeId,
    ) -> Result<DepositAddress, ServiceError> {
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

        let address_hash = sha256::Hash::hash(address.to_string().as_bytes());
        let address_hash_message = Message::from_digest(address_hash.to_byte_array());
        for operator in self.operator_pool.get_non_coordinator_operators() {
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

        Ok(DepositAddress {
            address,
            leaf_id: leaf_id.clone(),
            user_signing_public_key,
            verifying_public_key,
        })
    }
}
