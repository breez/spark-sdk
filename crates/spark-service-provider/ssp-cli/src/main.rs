#![cfg_attr(
    not(test),
    warn(
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::panic,
        clippy::string_slice,
        clippy::todo,
        clippy::unimplemented,
        clippy::unreachable,
        clippy::unwrap_used
    )
)]

use clap::{Parser, Subcommand};
use ssp_internal_api::{
    BalanceRequest, GetInfoRequest, GetLightningRequestRequest, ListPendingLightningRequest,
    NewAddressRequest, PoolStatusRequest, RequestRestockRequest, RestockDenomination, StopRequest,
    SubscribePoolEventsRequest, UtxosRequest, onchain_wallet_client::OnchainWalletClient,
    pool_client::PoolClient, pool_event, ssp_manager_client::SspManagerClient,
};
use tonic::{
    Request,
    transport::{Channel, Uri},
};

mod ssp_internal_api {
    tonic::include_proto!("ssp_internal");
}

#[derive(Parser)]
struct Args {
    /// Address to the internal grpc server.
    #[arg(long, default_value = "http://127.0.0.1:59050")]
    pub grpc_uri: Uri,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Get chain and network info
    GetInfo,
    /// Stop the sspd daemon
    Stop,
    /// Onchain wallet management
    Wallet {
        #[command(subcommand)]
        command: WalletCommand,
    },
    /// Inspect lightning send/receive state for debugging
    Lightning {
        #[command(subcommand)]
        command: LightningCommand,
    },
    /// Leaf pool: what it holds, what more to stock, what it is doing
    Pool {
        #[command(subcommand)]
        command: PoolCommand,
    },
}

#[derive(Subcommand)]
enum PoolCommand {
    /// Show the pool's leaves per denomination and anything still requested
    Status,
    /// Ask for more leaves of a denomination. The pool stocks them in the
    /// background, so this returns as soon as the daemon takes the ask, which a restart forgets;
    /// watch `events`
    /// to see it acted on. Fund the onchain wallet first if it is short:
    /// `wallet new-address` and send to it.
    Restock {
        /// Leaf value in sats. One of the pool's denominations.
        denomination_sats: u64,
        /// How many more leaves of it to stock.
        count: u32,
    },
    /// Follow what the pool service does as it stocks, until interrupted or the daemon stops
    Events,
}

#[derive(Subcommand)]
enum WalletCommand {
    /// Generate a new deposit address
    NewAddress,
    /// Get the onchain wallet balance
    Balance,
    /// List onchain UTXOs
    Utxos,
}

#[derive(Subcommand)]
enum LightningCommand {
    /// Look up a single send or receive by request id
    Get {
        /// The id the SSP returned when the send or receive was requested.
        id: String,
    },
    /// List all pending (not-yet-complete) sends and receives
    Pending,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    match args.command {
        Command::GetInfo => {
            let mut client = SspManagerClient::connect(args.grpc_uri).await?;
            let resp = client
                .get_info(Request::new(GetInfoRequest::default()))
                .await?
                .into_inner();
            println!("{}", serde_json::to_string_pretty(&resp)?);
        }
        Command::Stop => {
            let mut client = SspManagerClient::connect(args.grpc_uri).await?;
            client.stop(Request::new(StopRequest::default())).await?;
            println!("stop requested");
        }
        Command::Wallet { command } => {
            let client = OnchainWalletClient::connect(args.grpc_uri).await?;
            WalletHandler::new(client).execute(command).await?;
        }
        Command::Lightning { command } => {
            let client = SspManagerClient::connect(args.grpc_uri).await?;
            LightningHandler::new(client).execute(command).await?;
        }
        Command::Pool { command } => {
            let mut client = PoolClient::connect(args.grpc_uri).await?;
            match command {
                PoolCommand::Status => {
                    let status = client
                        .pool_status(Request::new(PoolStatusRequest {}))
                        .await?
                        .into_inner();
                    println!("available: {} sats", status.available_sats);
                    for leaves in status.available {
                        println!("  {:>8} sats x {}", leaves.denomination_sats, leaves.count);
                    }
                    if !status.pending_restock.is_empty() {
                        println!("requested, not yet planned:");
                        for request in status.pending_restock {
                            println!(
                                "  {:>8} sats x {}",
                                request.denomination_sats, request.count
                            );
                        }
                    }
                }
                PoolCommand::Restock {
                    denomination_sats,
                    count,
                } => {
                    let response = client
                        .request_restock(Request::new(RequestRestockRequest {
                            denominations: vec![RestockDenomination {
                                denomination_sats,
                                count,
                            }],
                        }))
                        .await?
                        .into_inner();
                    println!("requested {count} x {denomination_sats} sats");
                    for request in response.pending {
                        println!(
                            "  now pending: {:>8} sats x {}",
                            request.denomination_sats, request.count
                        );
                    }
                }
                PoolCommand::Events => {
                    let mut stream = client
                        .subscribe_pool_events(Request::new(SubscribePoolEventsRequest {}))
                        .await?
                        .into_inner();
                    while let Some(event) = stream.message().await? {
                        match event.event {
                            Some(pool_event::Event::FundingBroadcast(broadcast)) => println!(
                                "funding broadcast {} ({} sats, denominations {:?})",
                                broadcast.txid, broadcast.total_sats, broadcast.denominations
                            ),
                            Some(pool_event::Event::LeavesAvailable(leaves)) => println!(
                                "leaves available from {} ({:?})",
                                leaves.deposit_address, leaves.denominations
                            ),
                            None => println!("(unrecognised pool event)"),
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

struct WalletHandler {
    client: OnchainWalletClient<Channel>,
}

impl WalletHandler {
    fn new(client: OnchainWalletClient<Channel>) -> Self {
        Self { client }
    }

    async fn execute(&mut self, command: WalletCommand) -> Result<(), Box<dyn std::error::Error>> {
        match command {
            WalletCommand::NewAddress => {
                let resp = self
                    .client
                    .new_address(Request::new(NewAddressRequest {}))
                    .await?
                    .into_inner();
                println!("{}", resp.address);
            }
            WalletCommand::Balance => {
                let resp = self
                    .client
                    .balance(Request::new(BalanceRequest {}))
                    .await?
                    .into_inner();
                println!("{} sats ({} utxos)", resp.confirmed_sats, resp.utxo_count);
            }
            WalletCommand::Utxos => {
                let resp = self
                    .client
                    .utxos(Request::new(UtxosRequest {}))
                    .await?
                    .into_inner();
                println!("{}", serde_json::to_string_pretty(&resp)?);
            }
        }

        Ok(())
    }
}

struct LightningHandler {
    client: SspManagerClient<Channel>,
}

impl LightningHandler {
    fn new(client: SspManagerClient<Channel>) -> Self {
        Self { client }
    }

    async fn execute(
        &mut self,
        command: LightningCommand,
    ) -> Result<(), Box<dyn std::error::Error>> {
        match command {
            LightningCommand::Get { id } => {
                let resp = self
                    .client
                    .get_lightning_request(Request::new(GetLightningRequestRequest { id }))
                    .await?
                    .into_inner();
                match (resp.send, resp.receive) {
                    (Some(send), _) => println!("{}", serde_json::to_string_pretty(&send)?),
                    (_, Some(receive)) => println!("{}", serde_json::to_string_pretty(&receive)?),
                    (None, None) => println!("not found"),
                }
            }
            LightningCommand::Pending => {
                let resp = self
                    .client
                    .list_pending_lightning(Request::new(ListPendingLightningRequest {}))
                    .await?
                    .into_inner();
                println!("{}", serde_json::to_string_pretty(&resp)?);
            }
        }

        Ok(())
    }
}
