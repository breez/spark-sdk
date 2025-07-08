use clap::Subcommand;
use spark_wallet::SparkAddress;

#[derive(Clone, Debug, Subcommand)]
pub enum Command {
    #[command(subcommand)]
    Deposit(DepositCommand),
    /// Prints the wallet's info.
    Info,
    /// Prints the wallet's available balance.
    Balance,
    #[command(subcommand)]
    Leaves(LeavesCommand),
    #[command(subcommand)]
    Lightning(LightningCommand),
    /// Prints the wallet's Spark address.
    SparkAddress,
    #[command(subcommand)]
    Transfer(TransferCommand),
}

#[derive(Clone, Debug, Subcommand)]
pub enum DepositCommand {
    /// Claim a deposit after it has been confirmed onchain.
    Claim {
        /// The transaction ID of the deposit transaction.
        txid: String,
    },
    /// Generate a new onchain deposit address.
    NewAddress,
    NewAddressAndClaim,
}

#[derive(Clone, Debug, Subcommand)]
pub enum LeavesCommand {
    /// List all leaves in the wallet.
    List,
}

#[derive(Clone, Debug, Subcommand)]
pub enum LightningCommand {
    /// Create a lightning invoice.
    CreateInvoice {
        amount_sat: u64,
        description: Option<String>,
    },
    /// Fetch a lightning receive payment.
    FetchReceivePayment { id: String },
    /// Fetch a lightning send fee estimate.
    FetchSendFeeEstimate { invoice: String },
    /// Fetch a lightning send payment.
    FetchSendPayment { id: String },
    /// Pay a lightning invoice.
    PayInvoice {
        invoice: String,
        max_fee_sat: Option<u64>,
    },
}

#[derive(Clone, Debug, Subcommand)]
pub enum TransferCommand {
    /// Claims all pending transfers
    ClaimPending,

    /// Lists all transfers
    List,

    /// Lists all pending transfers
    ListPending,

    /// Transfer funds to another wallet.
    Transfer {
        /// The amount to transfer in satoshis.
        amount_sat: u64,
        /// The receiver's Spark address.
        receiver_address: SparkAddress,
    },
}
