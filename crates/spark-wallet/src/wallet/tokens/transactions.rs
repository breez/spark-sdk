use spark::{
    services::{FreezeIssuerTokenResponse, TransferTokenOutput},
    token::{
        PreparedTokenPackage, PreparedTokenTransfer, SelectionStrategy, TokenOutputWithPrevOut,
    },
};

use crate::{TokenRecipient, wallet::*};

impl SparkWallet {
    /// Transfers tokens to other Spark users.
    ///
    /// All recipients are paid by one transaction, and they may mix Spark
    /// addresses with Spark invoices and span several token ids. A transaction
    /// that pays a Spark invoice is the exception: it must stay on one token id.
    /// Sats invoices are not accepted: a sats payment moves leaves rather than
    /// token outputs and cannot share a transaction, so use {@link
    /// fulfill_spark_invoice} for those.
    pub async fn transfer_tokens(
        &self,
        recipients: Vec<TokenRecipient>,
        selected_outputs: Option<Vec<TokenOutputWithPrevOut>>,
        selection_strategy: Option<SelectionStrategy>,
    ) -> Result<TokenTransaction, SparkWalletError> {
        let (outputs, execute_before_unix_micros) = self.resolve_recipients(recipients)?;

        let tx = self
            .token_service
            .transfer_tokens(
                outputs,
                selected_outputs,
                selection_strategy,
                execute_before_unix_micros,
            )
            .await?;
        Ok(tx)
    }

    pub async fn create_issuer_token(
        &self,
        name: &str,
        ticker: &str,
        decimals: u32,
        is_freezable: bool,
        max_supply: u128,
        extra_metadata: Option<Vec<u8>>,
    ) -> Result<TokenTransaction, SparkWalletError> {
        let token_transaction = self
            .token_service
            .create_issuer_token(
                name,
                ticker,
                decimals,
                is_freezable,
                max_supply,
                extra_metadata,
            )
            .await?;
        Ok(token_transaction)
    }

    pub async fn mint_issuer_token(
        &self,
        amount: u128,
    ) -> Result<TokenTransaction, SparkWalletError> {
        let token_transaction = self.token_service.mint_issuer_token(amount).await?;
        Ok(token_transaction)
    }

    pub async fn burn_issuer_token(
        &self,
        amount: u128,
        preferred_outputs: Option<Vec<TokenOutputWithPrevOut>>,
    ) -> Result<TokenTransaction, SparkWalletError> {
        let token_transaction = self
            .token_service
            .burn_issuer_token(amount, preferred_outputs, None)
            .await?;
        Ok(token_transaction)
    }

    pub async fn freeze_issuer_token(
        &self,
        spark_address: &SparkAddress,
    ) -> Result<FreezeIssuerTokenResponse, SparkWalletError> {
        Ok(self
            .token_service
            .freeze_issuer_token(spark_address, false)
            .await?)
    }

    pub async fn unfreeze_issuer_token(
        &self,
        spark_address: &SparkAddress,
    ) -> Result<FreezeIssuerTokenResponse, SparkWalletError> {
        Ok(self
            .token_service
            .freeze_issuer_token(spark_address, true)
            .await?)
    }

    /// Optimizes token outputs by consolidating them when there are more than the configured threshold.
    /// Processes one token at a time. Token identifier can be provided, otherwise one is automatically selected.
    /// Only optimizes if the number of outputs is greater than the configured threshold.
    pub async fn optimize_token_outputs(
        &self,
        token_identifier: Option<&str>,
    ) -> Result<(), SparkWalletError> {
        self.token_service
            .optimize_token_outputs(
                token_identifier,
                self.config
                    .token_outputs_optimization_options
                    .min_outputs_threshold,
                self.config
                    .token_outputs_optimization_options
                    .target_output_count,
            )
            .await?;
        Ok(())
    }

    /// Prepares a single package paying the given recipients, for external signing.
    ///
    /// The recipients may mix Spark addresses with Spark invoices and span
    /// several token ids. Sats invoices are not accepted: a sats payment moves
    /// leaves rather than token outputs and cannot share a transaction.
    pub async fn prepare_token_package(
        &self,
        recipients: Vec<TokenRecipient>,
        selected_outputs: Option<Vec<TokenOutputWithPrevOut>>,
        selection_strategy: Option<SelectionStrategy>,
    ) -> Result<PreparedTokenPackage, SparkWalletError> {
        let (outputs, execute_before_unix_micros) = self.resolve_recipients(recipients)?;
        let prepared = self
            .token_service
            .prepare_token_transfer(
                outputs,
                selected_outputs,
                selection_strategy,
                execute_before_unix_micros,
            )
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

    pub(crate) async fn pay_token_invoice(
        &self,
        invoice_str: &str,
        amount: Option<u128>,
    ) -> Result<FulfillSparkInvoiceResult, SparkWalletError> {
        let tx = self
            .transfer_tokens(
                vec![TokenRecipient::Invoice {
                    invoice: invoice_str.to_string(),
                    amount,
                }],
                None,
                None,
            )
            .await?;

        Ok(FulfillSparkInvoiceResult::TokenTransaction(Box::new(tx)))
    }

    /// Validates each recipient and turns it into the output that pays it, along
    /// with the deadline the transaction as a whole must meet.
    fn resolve_recipients(
        &self,
        recipients: Vec<TokenRecipient>,
    ) -> Result<(Vec<TransferTokenOutput>, Option<i64>), SparkWalletError> {
        if recipients.is_empty() {
            return Err(SparkWalletError::Generic(
                "No recipients provided".to_string(),
            ));
        }

        // The same invoice twice would attach twice and pay twice for one request.
        // Repeating a plain address is legitimate: it is two outputs to one payee.
        let mut seen = HashSet::new();
        for recipient in &recipients {
            if let TokenRecipient::Invoice { invoice, .. } = recipient
                && !seen.insert(invoice.as_str())
            {
                return Err(SparkWalletError::InvalidAddress(format!(
                    "Invoice appears more than once: {invoice}"
                )));
            }
        }

        let mut outputs = Vec::with_capacity(recipients.len());
        let mut deadlines = Vec::with_capacity(recipients.len());
        for recipient in recipients {
            let (output, execute_before) = match recipient {
                TokenRecipient::Address {
                    token_id,
                    amount,
                    receiver_address,
                } => (
                    TransferTokenOutput {
                        token_id,
                        amount,
                        receiver_address,
                        spark_invoice: None,
                    },
                    None,
                ),
                TokenRecipient::Invoice { invoice, amount } => {
                    let parsed = self.parse_and_validate_spark_invoice(&invoice)?;
                    token_output_from_invoice(&parsed, &invoice, amount)?
                }
            };
            outputs.push(output);
            deadlines.push(execute_before);
        }

        validate_invoiced_transaction_is_single_token(&outputs)?;

        if !self.config.self_payment_allowed
            && outputs
                .iter()
                .any(|o| o.receiver_address.identity_public_key == self.identity_public_key)
        {
            return Err(SparkWalletError::SelfPaymentNotAllowed);
        }

        Ok((outputs, earliest_execute_before(deadlines)))
    }
}

impl BackgroundProcessor {
    pub(crate) async fn run_token_output_optimization(
        &self,
        interval: Duration,
        mut cancellation_token: watch::Receiver<()>,
    ) {
        info!(
            "Starting automatic token output optimization with interval: {:?}",
            interval
        );

        let run_optimization = || async {
            debug!("Running automatic token output optimization");
            match self
                .token_service
                .optimize_token_outputs(
                    None,
                    self.token_outputs_optimization_options
                        .min_outputs_threshold,
                    self.token_outputs_optimization_options.target_output_count,
                )
                .await
            {
                Ok(_) => {
                    debug!("Automatic token output optimization completed successfully");
                }
                Err(e) => {
                    error!("Error during automatic token output optimization: {:?}", e);
                }
            }
        };

        // Run first optimization immediately
        run_optimization().await;

        loop {
            // Wait for the interval or cancellation
            tokio::select! {
                _ = tokio::time::sleep(interval) => {
                    run_optimization().await;
                }
                _ = cancellation_token.changed() => {
                    info!("Stopping automatic token output optimization");
                    break;
                }
            }
        }
    }
}

/// `None` when there is no expiry or the instant is out of `i64` micros range, leaving the transfer unbounded.
fn execute_before_from_expiry(expiry_time: Option<SystemTime>) -> Option<i64> {
    expiry_time.and_then(|t| {
        t.duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|d| i64::try_from(d.as_micros()).ok())
    })
}

/// Folds per-invoice deadlines into the single transaction-level one, taking the
/// earliest: the transaction must land before the first invoice expires.
fn earliest_execute_before(deadlines: impl IntoIterator<Item = Option<i64>>) -> Option<i64> {
    deadlines.into_iter().flatten().min()
}

fn token_output_from_invoice(
    invoice: &SparkAddress,
    invoice_str: &str,
    amount: Option<u128>,
) -> Result<(TransferTokenOutput, Option<i64>), SparkWalletError> {
    let Some(invoice_fields) = &invoice.spark_invoice_fields else {
        return Err(SparkWalletError::InvalidAddress(format!(
            "Invoice does not include Spark invoice fields: {invoice:?}"
        )));
    };
    let Some(SparkAddressPaymentType::TokensPayment(payment)) = &invoice_fields.payment_type else {
        return Err(SparkWalletError::InvalidAddress(
            "Invoice does not include a token payment type".to_string(),
        ));
    };
    let Some(token_identifier) = &payment.token_identifier else {
        return Err(SparkWalletError::InvalidAddress(
            "Token invoice does not include token identifier".to_string(),
        ));
    };
    let amount = payment.amount.or(amount).ok_or(SparkWalletError::Generic(
        "Amount is required when invoice does not include an amount".to_string(),
    ))?;
    Ok((
        TransferTokenOutput {
            token_id: token_identifier.clone(),
            amount,
            receiver_address: invoice.clone(),
            spark_invoice: Some(invoice_str.to_string()),
        },
        execute_before_from_expiry(invoice_fields.expiry_time),
    ))
}

/// Rejects a token transaction that attaches a Spark invoice and pays more than
/// one token.
///
/// The operators read such a transaction as paying the token id of its first
/// output, and require every attached invoice to name that same token, so a
/// mixed-token transaction carrying an invoice is rejected on broadcast. Failing
/// here names the reason and costs nothing.
fn validate_invoiced_transaction_is_single_token(
    outputs: &[TransferTokenOutput],
) -> Result<(), SparkWalletError> {
    if !outputs.iter().any(|o| o.spark_invoice.is_some()) {
        return Ok(());
    }

    let mut token_ids: Vec<&str> = outputs.iter().map(|o| o.token_id.as_str()).collect();
    token_ids.sort_unstable();
    token_ids.dedup();
    if token_ids.len() > 1 {
        return Err(SparkWalletError::ValidationError(format!(
            "A token transaction that pays a Spark invoice must use a single token, but {} were requested: {}",
            token_ids.len(),
            token_ids.join(", ")
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn earliest_execute_before_picks_the_soonest_deadline() {
        assert_eq!(
            earliest_execute_before([Some(30), Some(10), Some(20)]),
            Some(10)
        );
    }

    #[test]
    fn earliest_execute_before_ignores_unbounded_invoices() {
        assert_eq!(earliest_execute_before([None, Some(15), None]), Some(15));
    }

    #[test]
    fn earliest_execute_before_none_when_all_unbounded() {
        assert_eq!(earliest_execute_before([None, None]), None);
        assert_eq!(earliest_execute_before([]), None);
    }

    #[test]
    fn execute_before_from_expiry_none_when_absent() {
        assert_eq!(execute_before_from_expiry(None), None);
    }

    #[test]
    fn execute_before_from_expiry_converts_to_micros() {
        let expiry = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        assert_eq!(
            execute_before_from_expiry(Some(expiry)),
            Some(1_700_000_000_000_000)
        );
    }

    #[test]
    fn execute_before_from_expiry_none_before_epoch() {
        let before_epoch = UNIX_EPOCH - Duration::from_secs(1);
        assert_eq!(execute_before_from_expiry(Some(before_epoch)), None);
    }

    const TOKEN_A: &str = "btkn1xgrvjwey5ngcagvap2dzzvsy4uk8ua9x69k82dwvt5e7ef9drm9qztux87";
    const TOKEN_B: &str = "btkn1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq";

    fn token_output(token_id: &str, invoice: Option<&str>) -> TransferTokenOutput {
        const TEST_PUBKEY_BYTES: [u8; 33] = [
            3, 141, 37, 201, 160, 148, 226, 93, 184, 201, 131, 47, 222, 91, 55, 171, 38, 95, 13,
            248, 175, 190, 44, 132, 189, 75, 131, 204, 215, 82, 93, 167, 177,
        ];
        TransferTokenOutput {
            token_id: token_id.to_string(),
            amount: 100,
            receiver_address: SparkAddress {
                identity_public_key: PublicKey::from_slice(&TEST_PUBKEY_BYTES).unwrap(),
                network: spark::Network::Mainnet,
                spark_invoice_fields: None,
            },
            spark_invoice: invoice.map(ToString::to_string),
        }
    }

    #[test]
    fn several_tokens_are_allowed_without_an_invoice() {
        let outputs = [token_output(TOKEN_A, None), token_output(TOKEN_B, None)];
        assert!(validate_invoiced_transaction_is_single_token(&outputs).is_ok());
    }

    #[test]
    fn several_invoices_are_allowed_on_one_token() {
        let outputs = [
            token_output(TOKEN_A, Some("spark1first")),
            token_output(TOKEN_A, Some("spark1second")),
            token_output(TOKEN_A, None),
        ];
        assert!(validate_invoiced_transaction_is_single_token(&outputs).is_ok());
    }

    #[test]
    fn an_invoice_alongside_a_second_token_is_rejected() {
        // The other token is paid to an address: the operators read every created
        // output, not only the invoiced ones.
        let outputs = [
            token_output(TOKEN_A, Some("spark1invoice")),
            token_output(TOKEN_B, None),
        ];
        let result = validate_invoiced_transaction_is_single_token(&outputs);
        assert!(matches!(result, Err(SparkWalletError::ValidationError(_))));
    }

    #[test]
    fn invoices_of_two_tokens_are_rejected() {
        let outputs = [
            token_output(TOKEN_A, Some("spark1first")),
            token_output(TOKEN_B, Some("spark1second")),
        ];
        let result = validate_invoiced_transaction_is_single_token(&outputs);
        assert!(matches!(result, Err(SparkWalletError::ValidationError(_))));
    }
}
