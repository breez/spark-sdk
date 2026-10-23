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
use ssp_internal_api::ListDisputesRequest;
use ssp_internal_api::ListReservationsRequest;
use ssp_internal_api::{
    BalanceRequest, GetInfoRequest, GetLightningRequestRequest, ListPendingLightningRequest,
    ListRefundOutputsRequest, NewAddressRequest, PoolStatusRequest, RequestRestockRequest,
    RestockDenomination, StopRequest, SubscribePoolEventsRequest, SweepRefundOutputsRequest,
    UtxosRequest, onchain_wallet_client::OnchainWalletClient, pool_client::PoolClient, pool_event,
    ssp_manager_client::SspManagerClient,
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
    /// List the unspent refund outputs of the leaves the SSP exited unilaterally
    Exits,
    /// Sweep those refund outputs into the onchain wallet
    SweepExits {
        /// Fee rate in sat/vB. Without it, the daemon uses its own estimate.
        #[arg(long)]
        fee_rate: Option<u64>,
        /// Also sweep the outputs that are inputs of an unconfirmed sweep, to replace it.
        /// Bitcoin Core accepts the new sweep only at a higher fee rate.
        #[arg(long)]
        replace: bool,
    },
    /// List the unreserved leaves that only one of the SSP and the operators
    /// would use, with what each side has for them
    Disputes,
    /// List the transfers the SSP has pool leaves reserved for
    Reservations {
        /// Only those that belong to no swap, lightning receive or static deposit claim.
        #[arg(long)]
        orphaned: bool,
    },
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
                PoolCommand::Exits => list_refund_outputs(&mut client).await?,
                PoolCommand::SweepExits { fee_rate, replace } => {
                    sweep_refund_outputs(&mut client, fee_rate, replace).await?;
                }
                PoolCommand::Disputes => list_disputes(&mut client).await?,
                PoolCommand::Reservations { orphaned } => {
                    list_reservations(&mut client, orphaned).await?;
                }
            }
        }
    }

    Ok(())
}

async fn list_disputes(client: &mut PoolClient<Channel>) -> Result<(), Box<dyn std::error::Error>> {
    let response = client
        .list_disputes(Request::new(ListDisputesRequest {}))
        .await?
        .into_inner();
    println!("{}", serde_json::to_string_pretty(&response)?);
    Ok(())
}

async fn list_reservations(
    client: &mut PoolClient<Channel>,
    orphaned_only: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let response = client
        .list_reservations(Request::new(ListReservationsRequest { orphaned_only }))
        .await?
        .into_inner();
    println!("{}", serde_json::to_string_pretty(&response)?);
    Ok(())
}

async fn list_refund_outputs(
    client: &mut PoolClient<Channel>,
) -> Result<(), Box<dyn std::error::Error>> {
    let outputs = client
        .list_refund_outputs(Request::new(ListRefundOutputsRequest {}))
        .await?
        .into_inner()
        .outputs;
    let total: u64 = outputs.iter().map(|output| output.value_sats).sum();
    for output in &outputs {
        let spender = output
            .spending_txid
            .as_ref()
            .map(|txid| format!(", an input of unconfirmed {txid}"))
            .unwrap_or_default();
        println!(
            "{:>8} sats  {}:{}  leaf {}, confirmed at {}{spender}",
            output.value_sats, output.txid, output.vout, output.leaf_id, output.block_height
        );
    }
    println!("{} outputs, {total} sats", outputs.len());
    Ok(())
}

async fn sweep_refund_outputs(
    client: &mut PoolClient<Channel>,
    fee_rate: Option<u64>,
    replace: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let sweep = client
        .sweep_refund_outputs(Request::new(SweepRefundOutputsRequest {
            fee_rate_sat_per_vbyte: fee_rate,
            replace_unconfirmed: replace,
        }))
        .await?
        .into_inner();
    for txid in &sweep.txids {
        println!("{txid}");
    }
    println!(
        "swept {} outputs: {} sats to the wallet, {} sats in fees",
        sweep.output_count, sweep.value_sats, sweep.fee_sats
    );
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::ssp_internal_api::GetInfoResponse;

    const TIP: &str = "5b2a796358296bfc94e7dc9376bbf1016b0588e93f41867c40e2101b3cd8082c";

    #[test]
    fn get_info_prints_the_block_sspd_is_at_and_whether_it_is_synced() {
        let synced = GetInfoResponse {
            network: "regtest".to_string(),
            block_height: Some(192),
            block_hash: Some(TIP.to_string()),
            synced: true,
            bitcoind_block_height: None,
            bitcoind_synced: None,
        };
        assert_eq!(
            serde_json::to_value(&synced).unwrap(),
            json!({
                "network": "regtest",
                "block_height": 192,
                "block_hash": TIP,
                "synced": true,
                "bitcoind_block_height": null,
                "bitcoind_synced": null,
            })
        );

        let behind = GetInfoResponse {
            block_height: Some(190),
            synced: false,
            bitcoind_block_height: Some(192),
            bitcoind_synced: Some(true),
            ..synced
        };
        assert_eq!(
            serde_json::to_value(&behind).unwrap(),
            json!({
                "network": "regtest",
                "block_height": 190,
                "block_hash": TIP,
                "synced": false,
                "bitcoind_block_height": 192,
                "bitcoind_synced": true,
            })
        );
    }
}
