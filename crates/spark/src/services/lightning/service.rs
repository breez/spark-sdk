use crate::operator::rpc::SparkRpcClient;
use crate::services::{
    LeafKeyTweak, ServiceError, from_proto_signing_commitments, to_proto_signed_tx,
};
use crate::ssp::ServiceProvider;
use crate::utils::refund as refund_utils;
use crate::{Network, signer::Signer, tree::TreeNode};
use bitcoin::hashes::{Hash, HashEngine, sha256};
use bitcoin::secp256k1::PublicKey;
use frost_secp256k1_tr::Identifier;
use lightning_invoice::Bolt11Invoice;
use spark_protos::spark::initiate_preimage_swap_request::Reason;
use spark_protos::spark::{
    GetSigningCommitmentsRequest, InitiatePreimageSwapRequest, InitiatePreimageSwapResponse,
    InvoiceAmount, InvoiceAmountProof, StartUserSignedTransferRequest,
};
use std::collections::BTreeMap;
use std::str::FromStr;
use std::sync::Arc;
use uuid::Uuid;

pub struct LightningService<S>
where
    S: Signer,
{
    spark_client: Arc<SparkRpcClient<S>>,
    ssp_client: Arc<ServiceProvider<S>>,
    network: Network,
    signer: S,
}

impl<S> LightningService<S>
where
    S: Signer,
{
    pub fn new(
        spark_client: Arc<SparkRpcClient<S>>,
        ssp_client: Arc<ServiceProvider<S>>,
        network: Network,
        signer: S,
    ) -> Self {
        LightningService {
            spark_client,
            ssp_client,
            network,
            signer,
        }
    }

    pub async fn pay_lightning_invoice(
        &self,
        invoice: &String,
        leaves: Vec<TreeNode>,
    ) -> Result<Uuid, ServiceError> {
        let invoice = Bolt11Invoice::from_str(invoice)
            .map_err(|err| ServiceError::InvoiceDecodingError(err.to_string()))?;

        // get the invoice amount in sats, then validate the amount
        let amount_sats = get_invoice_amount_sats(&invoice)?;
        if amount_sats == 0 {
            return Err(ServiceError::ValidationError(
                "Amount must be greater than 0".to_string(),
            ));
        }

        let leaves_amount: u64 = leaves.iter().map(|l| l.value).sum();
        if leaves_amount != amount_sats {
            return Err(ServiceError::ValidationError(
                "Amount must match the invoice amount".to_string(),
            ));
        }

        // get the payment hash from the invoice
        let payment_hash = invoice.payment_hash();

        // prepare leaf tweaks
        let mut leaf_tweaks = Vec::with_capacity(leaves.len());
        for tree_node in &leaves {
            // hash the leaf id
            let mut engine = sha256::Hash::engine();
            engine.input(tree_node.id.as_bytes());
            let leaf_hash = sha256::Hash::from_engine(engine);

            let signing_public_key = self.signer.generate_public_key(Some(leaf_hash))?;
            let new_signing_public_key = self.signer.generate_public_key(None)?;
            // derive the signing key
            let leaf_tweak = LeafKeyTweak {
                node: tree_node.clone(),
                signing_public_key,
                new_signing_public_key,
            };
            leaf_tweaks.push(leaf_tweak);
        }

        let swap_response = self
            .swap_nodes_for_preimage(
                leaf_tweaks,
                &self.ssp_client.identity_public_key(),
                payment_hash,
                &invoice,
                amount_sats,
                0, // TODO: this must use the estimated fee.
                false,
            )
            .await?;

        // let transfer = swap_response
        //     .transfer
        //     .ok_or(SparkSdkError::from(ValidationError::InvalidInput {
        //         field: "Swap response did not contain a transfer".to_string(),
        //     }))?
        //     .try_into()?;

        // // start the transfer
        // let transfer = self
        //     .send_transfer_tweak_key(&transfer, &leaf_tweaks, &HashMap::new())
        //     .await?;

        // // request Lightning send with the SSP
        // let lightning_send_response = self
        //     .request_lightning_send_with_ssp(invoice.to_string(), payment_hash.to_string())
        //     .await?;

        // // delete the leaves after the transfer
        // let leaf_ids_to_remove: Vec<String> = leaves.iter().map(|l| l.get_id().clone()).collect();
        // self.leaf_manager
        //     .unlock_leaves(unlocking_id.clone(), &leaf_ids_to_remove, true)?;
        todo!()
        //Ok(transfer.id)
    }

    // pub async fn create_lightning_invoice(
    //     &self,
    //     amount_sats: u64,
    //     memo: Option<String>,
    //     expiry_seconds: Option<i32>,
    // ) -> Result<Bolt11Invoice, SparkSdkError> {
    //     // default expiry to 30 days
    //     let expiry_seconds = expiry_seconds.unwrap_or(60 * 60 * 24 * 30);

    //     // generate the preimage
    //     // hash the preimage to get the payment hash
    //     let preimage_sk = bitcoin::secp256k1::SecretKey::new(&mut OsRng);
    //     let preimage_bytes = preimage_sk.secret_bytes();
    //     let payment_hash = sha256::digest(&preimage_bytes);
    //     let payment_hash_bytes = hex::decode(&payment_hash)
    //         .map_err(|err| SparkSdkError::from(IoError::Decoding(err)))?;

    //     // create the invoice by making a request to the SSP
    //     // TODO: we'll need to use fees
    //     let (invoice, _fees) = self
    //         .create_invoice_with_ssp(
    //             amount_sats,
    //             payment_hash,
    //             expiry_seconds,
    //             memo,
    //             self.config.spark_config.network,
    //         )
    //         .await?;

    //     // distribute the preimage shares to the operators
    //     // TODO: parallelize this
    //     let t = self.config.spark_config.threshold as usize;
    //     let n = self.config.spark_config.operator_pool.operators.len();
    //     let shares = self.signer.split_with_verifiable_secret_sharing(
    //         preimage_sk.secret_bytes().to_vec(),
    //         t,
    //         n,
    //     )?;

    //     let signing_operators = self.config.spark_config.operator_pool.operators.clone();
    //     let identity_pubkey = self.get_spark_address()?;

    //     let futures = signing_operators.iter().map(|operator| {
    //         let operator_id = operator.id;
    //         let share = &shares[operator_id as usize];
    //         let payment_hash = payment_hash_bytes.clone();
    //         let invoice_str = invoice.clone();
    //         let threshold = self.config.spark_config.threshold;
    //         let config = self.config.clone();

    //         async move {
    //             let request_data = StorePreimageShareRequest {
    //                 payment_hash,
    //                 preimage_share: Some(share.marshal_proto()),
    //                 threshold,
    //                 invoice_string: invoice_str,
    //                 user_identity_public_key: identity_pubkey.serialize().to_vec(),
    //             };

    //             config
    //                 .spark_config
    //                 .call_with_retry(
    //                     request_data,
    //                     |mut client, req| {
    //                         Box::pin(async move { client.store_preimage_share(req).await })
    //                     },
    //                     Some(operator_id),
    //                 )
    //                 .await
    //                 .map_err(|e| tonic::Status::internal(format!("RPC error: {}", e)))?;

    //             Ok(())
    //         }
    //     });

    //     futures::future::try_join_all(futures)
    //         .await
    //         .map_err(|e| SparkSdkError::from(NetworkError::Status(e)))?;

    //     Bolt11Invoice::from_str(&invoice).map_err(|err| {
    //         SparkSdkError::from(ValidationError::InvalidBolt11Invoice(err.to_string()))
    //     })
    // }

    async fn swap_nodes_for_preimage(
        &self,
        leaves: Vec<LeafKeyTweak>,
        receiver_pubkey: &PublicKey,
        payment_hash: &sha256::Hash,
        invoice: &Bolt11Invoice,
        invoice_amount_sats: u64,
        fee_sats: u64,
        is_inbound_payment: bool,
    ) -> Result<InitiatePreimageSwapResponse, ServiceError> {
        // get signing commitments
        let node_ids: Vec<String> = leaves.iter().map(|l| l.node.id.clone()).collect();
        let spark_commitments = self
            .spark_client
            .get_signing_commitments(GetSigningCommitmentsRequest { node_ids })
            .await?;

        // get user signed refunds
        let signing_commitments: Vec<
            BTreeMap<Identifier, frost_secp256k1_tr::round1::SigningCommitments>,
        > = spark_commitments
            .signing_commitments
            .iter()
            .map(|sc| from_proto_signing_commitments(sc.signing_nonce_commitments.clone()))
            .collect::<Result<Vec<_>, _>>()?;

        let user_signed_refunds = refund_utils::sign_refunds(
            &self.signer,
            leaves,
            signing_commitments,
            receiver_pubkey,
            self.network,
        )
        .await?;

        let transfer_id = Uuid::now_v7().to_string();
        let reason = if is_inbound_payment {
            Reason::Receive
        } else {
            Reason::Send
        };

        let request_data = InitiatePreimageSwapRequest {
            payment_hash: payment_hash.to_byte_array().to_vec(),
            reason: reason as i32,
            invoice_amount: Some(InvoiceAmount {
                invoice_amount_proof: Some(InvoiceAmountProof {
                    bolt11_invoice: invoice.to_string(),
                }),
                value_sats: invoice_amount_sats,
            }),
            transfer: Some(StartUserSignedTransferRequest {
                transfer_id: transfer_id.clone(),
                owner_identity_public_key: self
                    .signer
                    .get_identity_public_key(0)?
                    .serialize()
                    .to_vec(),
                receiver_identity_public_key: receiver_pubkey.serialize().to_vec(),
                expiry_time: Default::default(),
                leaves_to_send: user_signed_refunds
                    .iter()
                    .map(|l| to_proto_signed_tx(l))
                    .collect::<Result<Vec<_>, _>>()?,
            }),
            receiver_identity_public_key: receiver_pubkey.serialize().to_vec(),
            fee_sats,
        };

        let response = self
            .spark_client
            .initiate_preimage_swap(request_data)
            .await?;

        Ok(response)
    }
}

fn get_invoice_amount_sats(invoice: &Bolt11Invoice) -> Result<u64, ServiceError> {
    let invoice_amount_msats = invoice
        .amount_milli_satoshis()
        .ok_or(ServiceError::InvoiceDecodingError(invoice.to_string()))?;

    Ok(invoice_amount_msats / 1000)
}
