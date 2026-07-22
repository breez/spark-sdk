use super::*;

impl SparkWallet {
    /// Like the normal send path's selection, cancels an in-progress leaf
    /// optimization holding the leaves and retries once before failing with
    /// insufficient funds.
    async fn select_leaves_for_package_with_optimization_retry(
        &self,
        target_amounts: &TargetAmounts,
    ) -> Result<LeafSelection, SparkWalletError> {
        use spark::tree::TreeServiceError;

        match self
            .tree_service
            .select_leaves_for_package(Some(target_amounts))
            .await
        {
            Err(TreeServiceError::InsufficientFunds) if self.leaf_optimizer.is_running() => {
                debug!(
                    "Insufficient funds for package with optimization in progress, cancelling optimization and retrying"
                );
                if let Err(e) = self.leaf_optimizer.cancel().await {
                    debug!("Failed to cancel optimization: {e:?}");
                }
                Ok(self
                    .tree_service
                    .select_leaves_for_package(Some(target_amounts))
                    .await?)
            }
            res => Ok(res?),
        }
    }

    pub async fn prepare_transfer_package(
        &self,
        amount_sat: u64,
        receiver_address: &SparkAddress,
        spark_invoice: Option<&str>,
        transfer_id: Option<TransferId>,
    ) -> Result<SendPackagePreparation, SparkWalletError> {
        // Validate the invoice (expiry, sender) at build time, before the user is
        // asked to sign, matching the token path and publish_transfer_package.
        if let Some(invoice) = spark_invoice {
            self.parse_and_validate_spark_invoice(invoice)?;
        }
        if self.config.network != receiver_address.network {
            return Err(SparkWalletError::InvalidNetwork);
        }
        if !self.config.self_payment_allowed
            && receiver_address.identity_public_key == self.identity_public_key
        {
            return Err(SparkWalletError::SelfPaymentNotAllowed);
        }

        let target_amounts = TargetAmounts::new_amount_and_fee(amount_sat, None);
        match self
            .select_leaves_for_package_with_optimization_retry(&target_amounts)
            .await?
        {
            LeafSelection::Exact(leaves) => {
                let transfer_id = transfer_id.unwrap_or_else(TransferId::generate);
                let prepare_transfer = self.transfer_service.build_transfer_approval_request(
                    &transfer_id,
                    &leaves,
                    &receiver_address.identity_public_key,
                );
                Ok(SendPackagePreparation::Ready(prepare_transfer))
            }
            LeafSelection::SwapNeeded(leaves) => {
                let swap_targets = vec![amount_sat];
                let prepare_transfer = self
                    .swap_service
                    .prepare_swap(&leaves, Some(swap_targets.clone()))?;
                Ok(SendPackagePreparation::SwapRequired {
                    prepare_transfer,
                    target_amounts: swap_targets,
                })
            }
        }
    }

    pub async fn publish_transfer_package(
        &self,
        transfer_id: TransferId,
        receiver_public_key: PublicKey,
        leaf_ids: Vec<TreeNodeId>,
        spark_invoice: Option<String>,
        approved_transfer: PreparedTransfer,
    ) -> Result<WalletTransfer, SparkWalletError> {
        if let Some(invoice_str) = &spark_invoice {
            self.parse_and_validate_spark_invoice(invoice_str)?;
        }

        let reservation = self
            .tree_service
            .reserve_leaves_by_ids(&leaf_ids, ReservationPurpose::Payment)
            .await?;

        let transfer = with_reserved_leaves(
            self.tree_service.as_ref(),
            self.transfer_service.submit_transfer_with_prepared(
                &transfer_id,
                &reservation.leaves,
                &receiver_public_key,
                approved_transfer,
                spark_invoice,
            ),
            &reservation,
        )
        .await?;

        self.maybe_start_optimization().await;

        Ok(WalletTransfer::from_transfer(
            transfer,
            None,
            None,
            self.identity_public_key,
            self.config.service_provider_config.identity_public_key,
        ))
    }

    pub async fn publish_swap_package(
        &self,
        transfer_id: TransferId,
        leaf_ids: Vec<TreeNodeId>,
        target_amounts: Vec<u64>,
        approved_transfer: PreparedTransfer,
    ) -> Result<(), SparkWalletError> {
        // Attempt the swap optimistically. On a retry or a crash after submit the
        // leaves are already consumed, so this fails; the swap's primary transfer
        // is created under transfer_id at the operator, so if a transfer with this
        // id already exists the swap completed and we report success idempotently
        // instead of the leaf error. The happy path pays no extra query.
        match self
            .submit_swap_package_inner(
                transfer_id.clone(),
                leaf_ids,
                target_amounts,
                approved_transfer,
            )
            .await
        {
            Ok(()) => Ok(()),
            Err(e) => {
                if self
                    .transfer_service
                    .query_transfer(&transfer_id)
                    .await?
                    .is_some()
                {
                    return Ok(());
                }
                Err(e)
            }
        }
    }

    async fn submit_swap_package_inner(
        &self,
        transfer_id: TransferId,
        leaf_ids: Vec<TreeNodeId>,
        target_amounts: Vec<u64>,
        approved_transfer: PreparedTransfer,
    ) -> Result<(), SparkWalletError> {
        let reservation = self
            .tree_service
            .reserve_leaves_by_ids(&leaf_ids, ReservationPurpose::Swap)
            .await?;

        let claimed = match self
            .swap_service
            .submit_swap(
                transfer_id,
                &reservation.leaves,
                Some(target_amounts),
                approved_transfer,
            )
            .await
        {
            Ok(nodes) => nodes,
            Err(e) => {
                let _ = self.tree_service.cancel_reservation(reservation).await;
                return Err(e.into());
            }
        };

        if let Err(e) = self
            .tree_service
            .finalize_reservation(reservation.id.clone(), Some(claimed.as_slice()))
            .await
        {
            error!("Failed to finalize reservation: {e:?}");
        }
        self.maybe_start_optimization().await;
        Ok(())
    }

    pub async fn prepare_lightning_send_package(
        &self,
        invoice: &str,
        amount_to_send: Option<u64>,
        max_fee_sat: Option<u64>,
        transfer_id: Option<TransferId>,
    ) -> Result<SendPackagePreparation, SparkWalletError> {
        let (total_amount_sat, _) = self
            .lightning_service
            .validate_payment(invoice, max_fee_sat, amount_to_send, false)
            .await?;

        let target_amounts = TargetAmounts::new_amount_and_fee(total_amount_sat, None);
        match self
            .select_leaves_for_package_with_optimization_retry(&target_amounts)
            .await?
        {
            LeafSelection::Exact(leaves) => {
                let prepare_transfer = self
                    .lightning_service
                    .prepare_lightning_send(&leaves, transfer_id);
                Ok(SendPackagePreparation::Ready(prepare_transfer))
            }
            LeafSelection::SwapNeeded(leaves) => {
                let swap_targets = vec![total_amount_sat];
                let prepare_transfer = self
                    .swap_service
                    .prepare_swap(&leaves, Some(swap_targets.clone()))?;
                Ok(SendPackagePreparation::SwapRequired {
                    prepare_transfer,
                    target_amounts: swap_targets,
                })
            }
        }
    }

    pub async fn publish_lightning_send_package(
        &self,
        transfer_id: TransferId,
        leaf_ids: Vec<TreeNodeId>,
        invoice: String,
        amount_to_send: Option<u64>,
        approved_transfer: PreparedTransfer,
    ) -> Result<PayLightningInvoiceResult, SparkWalletError> {
        let reservation = self
            .tree_service
            .reserve_leaves_by_ids(&leaf_ids, ReservationPurpose::Payment)
            .await?;

        let lightning_payment = with_reserved_leaves(
            self.tree_service.as_ref(),
            self.lightning_service.submit_lightning_send(
                transfer_id,
                &reservation.leaves,
                &invoice,
                amount_to_send,
                approved_transfer,
            ),
            &reservation,
        )
        .await?;

        self.finalize_pay_lightning(lightning_payment).await
    }

    pub async fn prepare_coop_exit_package(
        &self,
        withdrawal_address: &str,
        amount_sats: u64,
        exit_speed: ExitSpeed,
        fee_quote: CoopExitFeeQuote,
        transfer_id: Option<TransferId>,
    ) -> Result<SendPackagePreparation, SparkWalletError> {
        withdrawal_address
            .parse::<Address<NetworkUnchecked>>()
            .map_err(|_| {
                SparkWalletError::InvalidAddress(format!(
                    "Invalid withdrawal address: {withdrawal_address}"
                ))
            })?
            .require_network(self.config.network.into())
            .map_err(|_| SparkWalletError::InvalidNetwork)?;

        let fee_sats = fee_quote.fee_sats(&exit_speed);
        let target_amounts = TargetAmounts::new_amount_and_fee(amount_sats, Some(fee_sats));
        match self
            .select_leaves_for_package_with_optimization_retry(&target_amounts)
            .await?
        {
            LeafSelection::Exact(leaves) => {
                let target_leaves =
                    select_leaves_by_target_amounts(&leaves, Some(&target_amounts))?;
                let ordered: Vec<TreeNode> = target_leaves
                    .amount_leaves
                    .into_iter()
                    .chain(target_leaves.fee_leaves.into_iter().flatten())
                    .collect();
                let prepare_transfer = self
                    .coop_exit_service
                    .prepare_coop_exit(&ordered, transfer_id);
                Ok(SendPackagePreparation::Ready(prepare_transfer))
            }
            LeafSelection::SwapNeeded(leaves) => {
                let swap_targets = vec![amount_sats, fee_sats];
                let prepare_transfer = self
                    .swap_service
                    .prepare_swap(&leaves, Some(swap_targets.clone()))?;
                Ok(SendPackagePreparation::SwapRequired {
                    prepare_transfer,
                    target_amounts: swap_targets,
                })
            }
        }
    }

    #[expect(clippy::too_many_arguments)]
    pub async fn publish_coop_exit_package(
        &self,
        transfer_id: TransferId,
        leaf_ids: Vec<TreeNodeId>,
        withdrawal_address: &str,
        amount_sats: u64,
        exit_speed: ExitSpeed,
        fee_quote: CoopExitFeeQuote,
        approved_transfer: PreparedTransfer,
    ) -> Result<WalletTransfer, SparkWalletError> {
        let withdrawal_address = withdrawal_address
            .parse::<Address<NetworkUnchecked>>()
            .map_err(|_| {
                SparkWalletError::InvalidAddress(format!(
                    "Invalid withdrawal address: {withdrawal_address}"
                ))
            })?
            .require_network(self.config.network.into())
            .map_err(|_| SparkWalletError::InvalidNetwork)?;

        let reservation = self
            .tree_service
            .reserve_leaves_by_ids(&leaf_ids, ReservationPurpose::Payment)
            .await?;

        let total_sats: u64 = reservation.leaves.iter().map(|leaf| leaf.value).sum();
        let Some(fee_sats) = total_sats.checked_sub(amount_sats) else {
            let _ = self.tree_service.cancel_reservation(reservation).await;
            return Err(SparkWalletError::Generic(
                "reserved leaves do not cover the amount".to_string(),
            ));
        };
        if fee_quote.fee_sats(&exit_speed) != fee_sats {
            let _ = self.tree_service.cancel_reservation(reservation).await;
            return Err(SparkWalletError::Generic(
                "reserved fee leaves do not match the requested confirmation speed".to_string(),
            ));
        }
        let target_amounts = TargetAmounts::new_amount_and_fee(amount_sats, Some(fee_sats));

        let target_leaves =
            match select_leaves_by_target_amounts(&reservation.leaves, Some(&target_amounts)) {
                Ok(t) => t,
                Err(e) => {
                    let _ = self.tree_service.cancel_reservation(reservation).await;
                    return Err(e.into());
                }
            };

        let transfer = match self
            .coop_exit_service
            .submit_coop_exit(
                CoopExitParams {
                    leaves: target_leaves.amount_leaves,
                    withdrawal_address: &withdrawal_address,
                    withdraw_all: false,
                    exit_speed,
                    fee_quote_id: Some(fee_quote.id.clone()),
                    fee_leaves: target_leaves.fee_leaves,
                    fee_sats,
                    transfer_id: Some(transfer_id.clone()),
                },
                approved_transfer,
            )
            .await
        {
            Ok(t) => t,
            Err(e) => {
                let _ = self.tree_service.cancel_reservation(reservation).await;
                return Err(e.into());
            }
        };

        if let Err(e) = self
            .tree_service
            .finalize_reservation(reservation.id.clone(), None)
            .await
        {
            error!("Failed to finalize reservation: {e:?}");
        }
        self.maybe_start_optimization().await;

        create_transfer(
            transfer,
            &self.ssp_client,
            &self.htlc_service,
            self.identity_public_key,
            self.config.service_provider_config.identity_public_key,
        )
        .await
    }

    pub async fn prepare_token_package(
        &self,
        outputs: Vec<TransferTokenOutput>,
        selected_outputs: Option<Vec<TokenOutputWithPrevOut>>,
        selection_strategy: Option<SelectionStrategy>,
    ) -> Result<PreparedTokenPackage, SparkWalletError> {
        if outputs.iter().any(|o| o.spark_invoice.is_some()) {
            return Err(SparkWalletError::Generic(
                "Spark invoices are not supported for token transfers. Use the `fulfill_spark_invoice` method instead.".to_string(),
            ));
        }

        if !self.config.self_payment_allowed
            && outputs
                .iter()
                .any(|o| o.receiver_address.identity_public_key == self.identity_public_key)
        {
            return Err(SparkWalletError::SelfPaymentNotAllowed);
        }

        let prepared = self
            .token_service
            .prepare_token_transfer(outputs, selected_outputs, selection_strategy, None)
            .await?;
        Ok(prepared)
    }

    pub async fn prepare_spark_invoice_token_package(
        &self,
        invoice_str: &str,
        amount: Option<u128>,
    ) -> Result<PreparedTokenPackage, SparkWalletError> {
        self.prepare_token_package_for_invoices(vec![SparkInvoiceToFulfill {
            invoice: invoice_str.to_string(),
            amount,
        }])
        .await
    }

    /// Prepares a single package paying the given token Spark invoices, for external
    /// signing.
    ///
    /// The invoices may request different tokens. Sats invoices are not accepted:
    /// a sats payment moves leaves rather than token outputs and cannot share a
    /// transaction.
    pub async fn prepare_token_package_for_invoices(
        &self,
        invoices: Vec<SparkInvoiceToFulfill>,
    ) -> Result<PreparedTokenPackage, SparkWalletError> {
        let (outputs, execute_before_unix_micros) = self.token_outputs_from_invoices(invoices)?;
        let prepared = self
            .token_service
            .prepare_token_transfer(outputs, None, None, execute_before_unix_micros)
            .await?;
        Ok(prepared)
    }

    pub async fn publish_token_package(
        &self,
        prepared: PreparedTokenTransfer,
        signature: Vec<u8>,
    ) -> Result<TokenTransaction, SparkWalletError> {
        for output in &prepared.receiver_outputs {
            if let Some(invoice) = &output.spark_invoice {
                self.parse_and_validate_spark_invoice(invoice)?;
            }
        }
        let tx = self
            .token_service
            .submit_token_transfer(prepared, signature)
            .await?;
        Ok(tx)
    }
}
