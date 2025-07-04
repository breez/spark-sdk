use clap::Subcommand;
use spark_wallet::SparkAddress;

#[derive(Clone, Debug, Subcommand)]
pub enum Command {
    /// Claim a deposit after it has been confirmed onchain.
    ClaimDeposit {
        /// The transaction ID of the deposit transaction.
        txid: String,
    },

    /// Generate a new onchain deposit address.
    GenerateDepositAddress,

    GenerateAndClaimDeposit,

    /// Pay a lightning invoice.
    PayLightningInvoice {
        invoice: String,
        max_fee_sat: Option<u64>,
    },

    /// Create a lightning invoice.
    CreateLightningInvoice {
        amount_sat: u64,
        description: Option<String>,
    },

    /// Fetch a lightning send fee estimate.
    FetchLightningSendFeeEstimate {
        invoice: String,
    },

    /// Fetch a lightning send payment.
    FetchLightningSendPayment {
        id: String,
    },

    /// Fetch a lightning receive payment.
    FetchLightningReceivePayment {
        id: String,
    },

    /// List all leaves in the wallet.
    ListLeaves,

    /// Prints the wallet's Spark address.
    SparkAddress,

    Transfer {
        /// The amount to transfer in satoshis.
        amount_sat: u64,
        /// The receiver's Spark address.
        receiver_address: SparkAddress,
    },
}
