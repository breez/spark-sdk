use bitcoin::{
    Address, Transaction,
    address::NetworkUnchecked,
    hashes::{Hash, sha256},
    key::Secp256k1,
    secp256k1::PublicKey,
};
use thiserror::Error;

use crate::{
    Network,
    cryptography::subtract_public_keys,
    operator::rpc::{OperatorRpcError, SparkRpcClient},
    signer::Signer,
};
use spark_protos::spark::GenerateDepositAddressRequest;

#[derive(Debug, Error)]
pub enum DepositServiceError {
    #[error("invalid deposit address")]
    InvalidDepositAddress,
    #[error("invalid deposit address network")]
    InvalidDepositAddressNetwork,
    #[error("missing deposit address")]
    MissingDepositAddress,
    #[error("missing deposit address proof")]
    MissingDepositAddressProof,
    #[error("missing leaf id")]
    MissingLeafId,
    #[error("invalid deposit address proof")]
    InvalidDepositAddressProof,
    #[error("service connection error: {0}")]
    ServiceConnectionError(#[from] OperatorRpcError),
}

pub struct DepositService<S>
where
    S: Signer,
{
    client: SparkRpcClient<S>,
    identity_public_key: PublicKey,
    network: Network,
}

pub struct DepositAddress {
    pub address: Address,
    pub leaf_id: String,
    pub user_signing_public_key: PublicKey,
    pub verifying_public_key: PublicKey,
}

impl<S> DepositService<S>
where
    S: Signer,
{
    fn spark_network(&self) -> spark_protos::spark::Network {
        self.network.into()
    }

    pub fn new(
        client: SparkRpcClient<S>,
        identity_public_key: PublicKey,
        network: impl Into<Network>,
    ) -> Self {
        DepositService {
            client,
            identity_public_key,
            network: network.into(),
        }
    }

    pub async fn create_tree_root(
        &self,
        address: &DepositAddress,
        tx: Transaction,
        vout: usize,
    ) -> Result<(), DepositServiceError> {
        todo!()
        // Create a root tx
        // const rootTx = new Transaction();
        // const output = depositTx.getOutput(vout);
        // if (!output) {
        //   throw new ValidationError("Invalid deposit transaction output", {
        //     field: "vout",
        //     value: vout,
        //     expected: "Valid output index",
        //   });
        // }
        // const script = output.script;
        // const amount = output.amount;
        // if (!script || !amount) {
        //   throw new ValidationError("No script or amount found in deposit tx", {
        //     field: "output",
        //     value: output,
        //     expected: "Output with script and amount",
        //   });
        // }

        // rootTx.addInput({
        //   txid: getTxId(depositTx),
        //   index: vout,
        // });

        // rootTx.addOutput({
        //   script,
        //   amount,
        // });

        // const rootNonceCommitment =
        //   await this.config.signer.getRandomSigningCommitment();
        // const rootTxSighash = getSigHashFromTx(rootTx, 0, output);

        // // Create a refund tx
        // const refundTx = new Transaction();
        // const sequence = (1 << 30) | INITIAL_TIME_LOCK;
        // refundTx.addInput({
        //   txid: getTxId(rootTx),
        //   index: 0,
        //   sequence,
        // });

        // const refundP2trAddress = getP2TRAddressFromPublicKey(
        //   signingPubKey,
        //   this.config.getNetwork(),
        // );
        // const refundAddress = btc
        //   .Address(getNetwork(this.config.getNetwork()))
        //   .decode(refundP2trAddress);
        // const refundPkScript = btc.OutScript.encode(refundAddress);

        // refundTx.addOutput({
        //   script: refundPkScript,
        //   amount: amount,
        // });

        // const refundNonceCommitment =
        //   await this.config.signer.getRandomSigningCommitment();
        // const refundTxSighash = getSigHashFromTx(refundTx, 0, output);

        // const sparkClient = await this.connectionManager.createSparkClient(
        //   this.config.getCoordinatorAddress(),
        // );

        // let treeResp: StartDepositTreeCreationResponse;

        // try {
        //   treeResp = await sparkClient.start_deposit_tree_creation({
        //     identityPublicKey: await this.config.signer.getIdentityPublicKey(),
        //     onChainUtxo: {
        //       vout: vout,
        //       rawTx: depositTx.toBytes(true),
        //       network: this.config.getNetworkProto(),
        //     },
        //     rootTxSigningJob: {
        //       rawTx: rootTx.toBytes(),
        //       signingPublicKey: signingPubKey,
        //       signingNonceCommitment: rootNonceCommitment,
        //     },
        //     refundTxSigningJob: {
        //       rawTx: refundTx.toBytes(),
        //       signingPublicKey: signingPubKey,
        //       signingNonceCommitment: refundNonceCommitment,
        //     },
        //   });
        // } catch (error) {
        //   throw new NetworkError(
        //     "Failed to start deposit tree creation",
        //     {
        //       operation: "start_deposit_tree_creation",
        //       errorCount: 1,
        //       errors: error instanceof Error ? error.message : String(error),
        //     },
        //     error as Error,
        //   );
        // }

        // if (!treeResp.rootNodeSignatureShares?.verifyingKey) {
        //   throw new ValidationError("No verifying key found in tree response", {
        //     field: "verifyingKey",
        //     value: treeResp.rootNodeSignatureShares,
        //     expected: "Non-null verifying key",
        //   });
        // }

        // if (
        //   !treeResp.rootNodeSignatureShares.nodeTxSigningResult
        //     ?.signingNonceCommitments
        // ) {
        //   throw new ValidationError(
        //     "No signing nonce commitments found in tree response",
        //     {
        //       field: "nodeTxSigningResult.signingNonceCommitments",
        //       value: treeResp.rootNodeSignatureShares.nodeTxSigningResult,
        //       expected: "Non-null signing nonce commitments",
        //     },
        //   );
        // }

        // if (
        //   !treeResp.rootNodeSignatureShares.refundTxSigningResult
        //     ?.signingNonceCommitments
        // ) {
        //   throw new ValidationError(
        //     "No signing nonce commitments found in tree response",
        //     {
        //       field: "refundTxSigningResult.signingNonceCommitments",
        //     },
        //   );
        // }

        // if (
        //   !equalBytes(treeResp.rootNodeSignatureShares.verifyingKey, verifyingKey)
        // ) {
        //   throw new ValidationError("Verifying key mismatch", {
        //     field: "verifyingKey",
        //     value: treeResp.rootNodeSignatureShares.verifyingKey,
        //     expected: verifyingKey,
        //   });
        // }

        // const rootSignature = await this.config.signer.signFrost({
        //   message: rootTxSighash,
        //   publicKey: signingPubKey,
        //   privateAsPubKey: signingPubKey,
        //   verifyingKey,
        //   selfCommitment: rootNonceCommitment,
        //   statechainCommitments:
        //     treeResp.rootNodeSignatureShares.nodeTxSigningResult
        //       .signingNonceCommitments,
        //   adaptorPubKey: new Uint8Array(),
        // });

        // const refundSignature = await this.config.signer.signFrost({
        //   message: refundTxSighash,
        //   publicKey: signingPubKey,
        //   privateAsPubKey: signingPubKey,
        //   verifyingKey,
        //   selfCommitment: refundNonceCommitment,
        //   statechainCommitments:
        //     treeResp.rootNodeSignatureShares.refundTxSigningResult
        //       .signingNonceCommitments,
        //   adaptorPubKey: new Uint8Array(),
        // });

        // const rootAggregate = await this.config.signer.aggregateFrost({
        //   message: rootTxSighash,
        //   statechainSignatures:
        //     treeResp.rootNodeSignatureShares.nodeTxSigningResult.signatureShares,
        //   statechainPublicKeys:
        //     treeResp.rootNodeSignatureShares.nodeTxSigningResult.publicKeys,
        //   verifyingKey: treeResp.rootNodeSignatureShares.verifyingKey,
        //   statechainCommitments:
        //     treeResp.rootNodeSignatureShares.nodeTxSigningResult
        //       .signingNonceCommitments,
        //   selfCommitment: rootNonceCommitment,
        //   publicKey: signingPubKey,
        //   selfSignature: rootSignature!,
        //   adaptorPubKey: new Uint8Array(),
        // });

        // const refundAggregate = await this.config.signer.aggregateFrost({
        //   message: refundTxSighash,
        //   statechainSignatures:
        //     treeResp.rootNodeSignatureShares.refundTxSigningResult.signatureShares,
        //   statechainPublicKeys:
        //     treeResp.rootNodeSignatureShares.refundTxSigningResult.publicKeys,
        //   verifyingKey: treeResp.rootNodeSignatureShares.verifyingKey,
        //   statechainCommitments:
        //     treeResp.rootNodeSignatureShares.refundTxSigningResult
        //       .signingNonceCommitments,
        //   selfCommitment: refundNonceCommitment,
        //   publicKey: signingPubKey,
        //   selfSignature: refundSignature,
        //   adaptorPubKey: new Uint8Array(),
        // });

        // let finalizeResp: FinalizeNodeSignaturesResponse;
        // try {
        //   finalizeResp = await sparkClient.finalize_node_signatures({
        //     intent: SignatureIntent.CREATION,
        //     nodeSignatures: [
        //       {
        //         nodeId: treeResp.rootNodeSignatureShares.nodeId,
        //         nodeTxSignature: rootAggregate,
        //         refundTxSignature: refundAggregate,
        //       },
        //     ],
        //   });
        // } catch (error) {
        //   throw new NetworkError(
        //     "Failed to finalize node signatures",
        //     {
        //       operation: "finalize_node_signatures",
        //       errorCount: 1,
        //       errors: error instanceof Error ? error.message : String(error),
        //     },
        //     error as Error,
        //   );
        // }

        // return finalizeResp;
    }

    pub async fn generate_deposit_address(
        &self,
        signing_public_key: PublicKey,
        leaf_id: String,
        is_static: bool,
    ) -> Result<DepositAddress, DepositServiceError> {
        let resp = self
            .client
            .generate_deposit_address(GenerateDepositAddressRequest {
                signing_public_key: signing_public_key.serialize().to_vec(),
                identity_public_key: self.identity_public_key.serialize().to_vec(),
                network: self.spark_network() as i32,
                leaf_id: Some(leaf_id.clone()),
                is_static: Some(is_static),
            })
            .await?;

        let Some(deposit_address) = resp.deposit_address else {
            return Err(DepositServiceError::MissingDepositAddress);
        };

        let address =
            self.validate_deposit_address(deposit_address, signing_public_key, leaf_id)?;

        Ok(address)
    }

    pub async fn query_unused_deposit_addresses(
        &self,
    ) -> Result<Vec<DepositAddress>, DepositServiceError> {
        let resp = self
            .client
            .query_unused_deposit_addresses(
                spark_protos::spark::QueryUnusedDepositAddressesRequest {
                    identity_public_key: self.identity_public_key.serialize().to_vec(),
                    network: self.spark_network() as i32,
                },
            )
            .await?;

        let addresses = resp
            .deposit_addresses
            .into_iter()
            .map(|addr| {
                let address: Address<NetworkUnchecked> = addr
                    .deposit_address
                    .parse()
                    .map_err(|_| DepositServiceError::InvalidDepositAddress)?;

                Ok(DepositAddress {
                    address: address
                        .require_network(self.network.into())
                        .map_err(|_| DepositServiceError::InvalidDepositAddressNetwork)?,
                    // TODO: Is it possible addresses do not have a leaf_id?
                    leaf_id: addr.leaf_id.ok_or(DepositServiceError::MissingLeafId)?,
                    user_signing_public_key: PublicKey::from_slice(&addr.user_signing_public_key)
                        .map_err(|_| {
                        DepositServiceError::InvalidDepositAddressProof
                    })?,
                    verifying_public_key: PublicKey::from_slice(&addr.verifying_public_key)
                        .map_err(|_| DepositServiceError::InvalidDepositAddressProof)?,
                })
            })
            .collect::<Result<Vec<_>, DepositServiceError>>()
            .map_err(|_| DepositServiceError::InvalidDepositAddress)?;

        Ok(addresses)
    }

    fn proof_of_possession_message_hash(
        &self,
        operator_public_key: &PublicKey,
        address: &Address,
    ) -> sha256::Hash {
        let mut msg = operator_public_key.serialize().to_vec();
        msg.extend_from_slice(&self.identity_public_key.serialize());
        msg.extend_from_slice(address.to_string().as_bytes());
        sha256::Hash::hash(&msg)
    }

    fn validate_deposit_address(
        &self,
        deposit_address: spark_protos::spark::Address,
        signing_public_key: PublicKey,
        leaf_id: String,
    ) -> Result<DepositAddress, DepositServiceError> {
        let address: Address<NetworkUnchecked> = deposit_address
            .address
            .parse()
            .map_err(|_| DepositServiceError::InvalidDepositAddress)?;
        let address = address
            .require_network(self.network.into())
            .map_err(|e| DepositServiceError::InvalidDepositAddressNetwork)?;

        let Some(proof) = deposit_address.deposit_address_proof else {
            return Err(DepositServiceError::MissingDepositAddressProof);
        };

        let verifying_key = PublicKey::from_slice(&deposit_address.verifying_key)
            .map_err(|_| DepositServiceError::InvalidDepositAddressProof)?;

        // TODO: Move this in a separate service? Don't want to touch secp256k1 directly here.
        let secp = Secp256k1::new();
        let operator_public_key = subtract_public_keys(&verifying_key, &signing_public_key, &secp)
            .map_err(|_| DepositServiceError::InvalidDepositAddressProof)?;
        let msg = self.proof_of_possession_message_hash(&operator_public_key, &address);

        todo!()
        // const msg = proofOfPossessionMessageHashForDepositAddress(
        //   await this.config.signer.getIdentityPublicKey(),
        //   operatorPubkey,
        //   address.address,
        // );

        // const taprootKey = p2tr(
        //   operatorPubkey.slice(1, 33),
        //   undefined,
        //   getNetwork(this.config.getNetwork()),
        // ).tweakedPubkey;

        // const isVerified = schnorr.verify(
        //   address.depositAddressProof.proofOfPossessionSignature,
        //   msg,
        //   taprootKey,
        // );

        // if (!isVerified) {
        //   throw new ValidationError(
        //     "Proof of possession signature verification failed",
        //     {
        //       field: "proofOfPossessionSignature",
        //       value: address.depositAddressProof.proofOfPossessionSignature,
        //     },
        //   );
        // }

        // const addrHash = sha256(address.address);
        // for (const operator of Object.values(this.config.getSigningOperators())) {
        //   if (operator.identifier === this.config.getCoordinatorIdentifier()) {
        //     continue;
        //   }

        //   const operatorPubkey = hexToBytes(operator.identityPublicKey);
        //   const operatorSig =
        //     address.depositAddressProof.addressSignatures[operator.identifier];
        //   if (!operatorSig) {
        //     throw new ValidationError("Operator signature not found", {
        //       field: "addressSignatures",
        //       value: operator.identifier,
        //     });
        //   }
        //   const sig = secp256k1.Signature.fromDER(operatorSig);

        //   const isVerified = secp256k1.verify(sig, addrHash, operatorPubkey);
        //   if (!isVerified) {
        //     throw new ValidationError("Operator signature verification failed", {
        //       field: "operatorSignature",
        //       value: operatorSig,
        //     });
        //   }
        // }
    }
}
