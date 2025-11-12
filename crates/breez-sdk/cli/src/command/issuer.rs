use breez_sdk_spark::{
    BurnIssuerTokenRequest, CreateIssuerTokenRequest, FreezeIssuerTokenRequest,
    MintIssuerTokenRequest, TokenIssuer, UnfreezeIssuerTokenRequest,
};
use clap::{ArgAction, Subcommand};

use crate::command::print_value;

#[derive(Clone, Debug, Subcommand)]
pub enum IssuerCommand {
    /// Gets the issuer token balance
    TokenBalance,
    /// Gets the issuer token metadata
    TokenMetadata,
    /// Creates a new issuer token
    CreateToken {
        /// Name of the token
        name: String,
        /// Ticker symbol of the token
        ticker: String,
        /// Number of decimal places for the token
        decimals: u32,
        /// Whether the token is freezable
        #[arg(short = 'f', long, action = ArgAction::SetTrue)]
        is_freezable: bool,
        /// Maximum supply of the token
        max_supply: u128,
    },
    /// Mints supply of the issuer token
    MintToken {
        /// Amount of the supply to mint
        amount: u128,
    },
    /// Burns supply of the issuer token
    BurnToken {
        /// Amount of the supply to burn
        amount: u128,
    },
    /// Freezes issuer tokens held at the specified address
    FreezeToken {
        /// Address holding the tokens to freeze
        address: String,
    },
    /// Unfreezes issuer tokens held at the specified address
    UnfreezeToken {
        /// Address holding the tokens to unfreeze
        address: String,
    },
}

pub async fn handle_command(
    token_issuer: &TokenIssuer,
    command: IssuerCommand,
) -> Result<bool, anyhow::Error> {
    match command {
        IssuerCommand::TokenBalance => {
            let response = token_issuer.get_issuer_token_balance().await?;
            print_value(&response)?;
            Ok(true)
        }
        IssuerCommand::TokenMetadata => {
            let metadata = token_issuer.get_issuer_token_metadata().await?;
            print_value(&metadata)?;
            Ok(true)
        }
        IssuerCommand::CreateToken {
            name,
            ticker,
            decimals,
            is_freezable,
            max_supply,
        } => {
            let metadata = token_issuer
                .create_issuer_token(CreateIssuerTokenRequest {
                    name,
                    ticker,
                    decimals,
                    is_freezable,
                    max_supply,
                })
                .await?;
            print_value(&metadata)?;
            Ok(true)
        }
        IssuerCommand::MintToken { amount } => {
            let payment = token_issuer
                .mint_issuer_token(MintIssuerTokenRequest { amount })
                .await?;
            print_value(&payment)?;
            Ok(true)
        }
        IssuerCommand::BurnToken { amount } => {
            let payment = token_issuer
                .burn_issuer_token(BurnIssuerTokenRequest { amount })
                .await?;
            print_value(&payment)?;
            Ok(true)
        }
        IssuerCommand::FreezeToken { address } => {
            let response = token_issuer
                .freeze_issuer_token(FreezeIssuerTokenRequest { address })
                .await?;
            print_value(&response)?;
            Ok(true)
        }
        IssuerCommand::UnfreezeToken { address } => {
            let response = token_issuer
                .unfreeze_issuer_token(UnfreezeIssuerTokenRequest { address })
                .await?;
            print_value(&response)?;
            Ok(true)
        }
    }
}
