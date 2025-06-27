use clap::Subcommand;

#[derive(Clone, Debug, Subcommand)]
pub enum Command {
    /// Claim a deposit after it has been confirmed onchain.
    ClaimDeposit { address: String },

    /// Generate a new onchain deposit address.
    GenerateDepositAddress,
}
