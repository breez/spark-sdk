use breez_sdk_spark::{
    BreezSdk, BridgeRoutePair, BridgeRoutePairSide, CreateBridgeRequest, CreateBridgeResponse,
    FeePolicy, GetBridgeRoutesRequest, GetBridgeStatusRequest,
};
use clap::Subcommand;
use rustyline::{Editor, history::DefaultHistory};

use crate::command::{CliHelper, maybe_truncate_address, print_value, service_fee_denomination};

#[derive(Clone, Debug, Subcommand)]
pub enum BridgingCommand {
    /// Create a bridge that pays a recipient from another chain or over
    /// Lightning. Returns what the payer pays.
    Create {
        /// Who to pay: a Spark address, a Lightning address or Cash App
        /// `$cashtag`, or an EVM/Solana/Tron address
        recipient: String,

        /// Amount in stablecoin base units: the recipient's stablecoin when
        /// they are paid in one, the payer's otherwise. `1000000` = 1 USDC at
        /// 6 decimals, about $1
        #[arg(long)]
        amount: u128,

        /// The payer's address on the source chain, refunded if delivery
        /// fails. Required to pay a Lightning address
        #[arg(long)]
        refund_address: Option<String>,

        /// Deduct fees from the amount instead of adding them on top
        #[arg(long = "fees-included", action = clap::ArgAction::SetTrue)]
        fees_included: bool,

        /// Maximum slippage in basis points
        #[arg(long)]
        max_slippage_bps: Option<u32>,
    },
    /// Bridge from Cash App to USDC/USDT on an external chain. Returns a
    /// `cash.app` link for the payer to open.
    FromCashApp {
        /// Recipient address on the destination chain (EVM/Solana/Tron)
        recipient: String,

        /// Amount in the destination stablecoin's base units (6 decimals for
        /// USDC/USDT), so `1000000` = 1 USDC, about $1
        #[arg(long)]
        amount: u128,

        /// Deduct fees from the amount instead of adding them on top
        #[arg(long = "fees-included", action = clap::ArgAction::SetTrue)]
        fees_included: bool,

        /// Maximum slippage in basis points
        #[arg(long)]
        max_slippage_bps: Option<u32>,
    },
    /// Bridge USDC/USDT from an external chain to a Cash App user. Returns the
    /// deposit for the payer to make.
    ToCashApp {
        /// Cash App `$cashtag` or Cash App Lightning address
        recipient: String,

        /// Amount in the source stablecoin's base units (6 decimals for
        /// USDC/USDT), so `1000000` = 1 USDC, about $1
        #[arg(long)]
        amount: u128,

        /// The payer's address on the source chain, refunded if delivery fails
        #[arg(long)]
        refund_address: String,

        /// Deduct fees from the amount instead of adding them on top
        #[arg(long = "fees-included", action = clap::ArgAction::SetTrue)]
        fees_included: bool,

        /// Maximum slippage in basis points
        #[arg(long)]
        max_slippage_bps: Option<u32>,
    },
    /// Read a bridge's status
    Status {
        /// The bridge id `create` returned
        bridge_id: String,

        /// The status token `bridging create` returned
        #[arg(long)]
        status_token: Option<String>,
    },
}

#[allow(clippy::too_many_lines)]
pub async fn handle_command(
    rl: &mut Editor<CliHelper, DefaultHistory>,
    sdk: &BreezSdk,
    command: BridgingCommand,
) -> Result<bool, anyhow::Error> {
    let client = sdk.get_bridging_client()?;
    match command {
        BridgingCommand::Create {
            recipient,
            amount,
            refund_address,
            fees_included,
            max_slippage_bps,
        } => {
            let routes = client
                .get_routes(GetBridgeRoutesRequest {
                    recipient: recipient.clone(),
                })
                .await?
                .routes;
            let route = choose_bridge_route(rl, routes)?;
            let fee_policy = fees_included.then_some(FeePolicy::FeesIncluded);
            let response = client
                .create_bridge(CreateBridgeRequest {
                    recipient,
                    route,
                    amount,
                    refund_address,
                    fee_policy,
                    max_slippage_bps,
                })
                .await?;
            println!("Ask the payer to pay:");
            println!("{}", response.payment_request);
            print_value(&response)?;
            print_bridge_tracking(&response);
            Ok(true)
        }
        BridgingCommand::FromCashApp {
            recipient,
            amount,
            fees_included,
            max_slippage_bps,
        } => {
            let routes = client
                .get_routes(GetBridgeRoutesRequest {
                    recipient: recipient.clone(),
                })
                .await?
                .routes
                .into_iter()
                .filter(|route| route.source.chain == "lightning")
                .collect();
            let route = choose_bridge_route(rl, routes)?;
            let fee_policy = if fees_included {
                Some(FeePolicy::FeesIncluded)
            } else {
                None
            };
            let response = client
                .bridge_from_cash_app(CreateBridgeRequest {
                    recipient,
                    route,
                    amount,
                    refund_address: None,
                    fee_policy,
                    max_slippage_bps,
                })
                .await?;
            let info = &response.info;
            println!("Open this URL in a browser to complete the purchase:");
            println!("{}", response.payment_request);
            println!(
                "Deposit ~{} sats; recipient receives ~{} {}, service fee {} {}, expires {}",
                info.deposit_amount,
                info.expected_received_amount,
                info.destination_asset,
                info.service_fee_amount,
                service_fee_denomination(
                    info.service_fee_asset.as_deref(),
                    info.service_fee_asset_decimals,
                ),
                info.expires_at,
            );
            print_bridge_tracking(&response);
            Ok(true)
        }
        BridgingCommand::ToCashApp {
            recipient,
            amount,
            refund_address,
            fees_included,
            max_slippage_bps,
        } => {
            let routes = client
                .get_routes(GetBridgeRoutesRequest {
                    recipient: recipient.clone(),
                })
                .await?
                .routes;
            let route = choose_bridge_route(rl, routes)?;
            let fee_policy = if fees_included {
                Some(FeePolicy::FeesIncluded)
            } else {
                None
            };
            let response = client
                .bridge_to_cash_app(CreateBridgeRequest {
                    recipient,
                    route: route.clone(),
                    amount,
                    refund_address: Some(refund_address),
                    fee_policy,
                    max_slippage_bps,
                })
                .await?;
            let info = &response.info;
            println!("Ask the payer to pay:");
            println!("{}", response.payment_request);
            println!(
                "Deposit {} {} on {} to {}. Recipient receives ~{} sats, service fee {} {}, \
                 expires at {}",
                info.deposit_amount,
                route.source.asset,
                route.source.chain,
                info.deposit_address,
                info.expected_received_amount,
                info.service_fee_amount,
                service_fee_denomination(
                    info.service_fee_asset.as_deref(),
                    info.service_fee_asset_decimals,
                ),
                info.expires_at,
            );
            print_bridge_tracking(&response);
            Ok(true)
        }
        BridgingCommand::Status {
            bridge_id,
            status_token,
        } => {
            let response = client
                .get_bridge_status(GetBridgeStatusRequest {
                    bridge_id,
                    status_token,
                })
                .await?;
            print_value(&response)?;
            Ok(true)
        }
    }
}

/// Prompts the user to pick one of `routes`, auto-selecting a lone route.
pub(crate) fn choose_bridge_route(
    rl: &mut Editor<CliHelper, DefaultHistory>,
    routes: Vec<BridgeRoutePair>,
) -> Result<BridgeRoutePair, anyhow::Error> {
    if routes.is_empty() {
        return Err(anyhow::anyhow!("No bridge routes available"));
    }

    if routes.len() == 1 {
        let route = routes.into_iter().next().unwrap();
        println!("Auto-selected route: {}", route_label(&route));
        return Ok(route);
    }

    println!("Available bridge routes:");
    for (i, route) in routes.iter().enumerate() {
        println!("  {}. {}", i.saturating_add(1), route_label(route));
    }

    let line = rl.readline("Select route: ")?;
    let choice: usize = line
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid selection"))?;
    let index = choice
        .checked_sub(1)
        .ok_or_else(|| anyhow::anyhow!("Selection out of range"))?;
    routes
        .into_iter()
        .nth(index)
        .ok_or_else(|| anyhow::anyhow!("Selection out of range"))
}

fn route_label(route: &BridgeRoutePair) -> String {
    format!(
        "{} -> {} [{:?}]",
        side_label(&route.source),
        side_label(&route.destination),
        route.provider
    )
}

fn side_label(side: &BridgeRoutePairSide) -> String {
    format!(
        "{} on {}{}",
        side.asset,
        side.chain,
        maybe_truncate_address(side.contract_address.as_deref())
    )
}

/// Prints the `bridging status` command that follows the bridge.
pub(crate) fn print_bridge_tracking(response: &CreateBridgeResponse) {
    let token = response
        .status_token
        .as_deref()
        .map(|token| format!(" --status-token {token}"))
        .unwrap_or_default();
    println!(
        "Track it with: bridging status {}{token}",
        response.bridge_id
    );
}
