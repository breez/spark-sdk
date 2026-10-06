# Local Spark environment

A Spark regtest network of your own: bitcoind, three Spark operators, a Spark
service provider (SSP) with its Lightning node, an LNURL server, a data-sync
service, and the mempool explorer with its API.

```bash
docker compose -f regtest/local/docker-compose.yml up
```

It prints where its services are, and the Spark config a wallet connects with,
once the SSP can serve one. `make local-env-up` runs it in the background,
`make local-env-down` stops it, `make local-env-reset` deletes its state.

The same services run natively under Nix, as processes rather than containers:

```bash
nix run .#local-env
```

## Connecting a wallet

Docker writes the environment's Spark config to
`regtest/local/data/spark-config.json`, and Nix to `spark-config.json` in its
state directory. A wallet starts from the SDK's default regtest config and
changes five things:

- **Spark config**: `parse_spark_config` reads the file, which carries the
  operators and the SSP. Its result goes on `spark_config`.
- **Chain service**: a REST chain service at `http://127.0.0.1:8090/api`, of
  type `MempoolSpace`.
- **Deposit claim fee**: the SSP can quote more to claim a deposit than the
  default `max_deposit_claim_fee` of 1 sat/vbyte allows. A deposit quoted above
  the ceiling waits for a manual claim.
- **Lightning address domain**: `lnurl_domain` is the LNURL server,
  `http://127.0.0.1:8080`.
- **Sync server**: `real_time_sync_server_url` is the data-sync service,
  `http://127.0.0.1:8081`. The JavaScript SDK reaches it over gRPC-Web, at
  `http://127.0.0.1:8082`.

```rust
let mut config = default_config(Network::Regtest);
let spark_config = std::fs::read_to_string("regtest/local/data/spark-config.json")?;
config.spark_config = Some(parse_spark_config(spark_config)?);
config.max_deposit_claim_fee = Some(MaxFee::Rate { sat_per_vbyte: 5 });
config.lnurl_domain = Some("http://127.0.0.1:8080".to_string());
config.real_time_sync_server_url = Some("http://127.0.0.1:8081".to_string());
```

Every service a wallet uses is served over plain HTTP and answers cross-origin
requests, so a wallet in a browser connects the same way. For an Android
emulator, start the environment with `PUBLIC_HOST=10.0.2.2`. For a phone on your
network, set `PUBLIC_HOST` to your machine's address there and
`BIND_ADDRESS=0.0.0.0`.

## Settings

Every setting is an environment variable read when the environment starts:
`BITCOIND_RPC_PORT=18500 docker compose -f regtest/local/docker-compose.yml up`.

| Setting | Default | What it sets |
|---|---|---|
| `BITCOIND_RPC_PORT` | `18443` | Bitcoin Core's RPC port |
| `BITCOIND_P2P_PORT` | `18444` | Bitcoin Core's P2P port |
| `OPERATOR_0_PORT` to `OPERATOR_2_PORT` | `8535` to `8537` | The ports wallets reach the operators at, over plain HTTP |
| `SSP_PORT` | `59049` | The SSP's GraphQL port |
| `SSP_INTERNAL_PORT` | `59050` | The SSP's internal API, on loopback whatever `BIND_ADDRESS` says |
| `LDK_P2P_PORT` | `9735` | The SSP's Lightning node |
| `LDK_ALICE_P2P_PORT` | `9736` | Alice, the node it holds a channel with |
| `LNURL_PORT` | `8080` | The LNURL server, which serves lightning addresses |
| `DATA_SYNC_PORT` | `8081` | The data-sync service |
| `DATA_SYNC_WEB_PORT` | `8082` | The same service over gRPC-Web, which browsers reach it by |
| `MEMPOOL_PORT` | `8090` | The explorer, and the chain API under `/api` |
| `BIND_ADDRESS` | `127.0.0.1` | The address the published ports listen on |
| `PUBLIC_HOST` | `127.0.0.1` | The host wallets reach the environment by, in its Spark config |
| `BLOCK_INTERVAL_SECONDS` | `5` | Seconds between mined blocks. `0` mines until the environment is ready, then only on request |
| `CHANNEL_SATS` | `5000000000` | The channel Alice opens with the SSP's node, half of it pushed |
| `LEAVES_PER_DENOMINATION` | `8` | Leaves the SSP keeps of each denomination |
| `MAX_DENOMINATION_POWER` | `16` | Largest denomination the SSP keeps, in powers of two sats |
| `DKG_MIN_AVAILABLE_KEYS` | `12000` | Keyshares each operator keeps unused. The SSP's pool spends over 10,000 |
| `READY_TIMEOUT_SECONDS` | `2400` | How long the environment reports progress before it gives up |
| `SPARK_LOCAL_DIR` | `./.spark-local` | Where Nix keeps the environment's state |

Nix reads a few more, for the ports its services hold to one host:
`OPERATOR_0_TLS_PORT` to `OPERATOR_2_TLS_PORT`, `POSTGRES_PORT`,
`ELECTRS_PORT`, `ELECTRS_ELECTRUM_PORT`, `ELECTRS_MONITORING_PORT`,
`MEMPOOL_API_PORT`, `LDK_GRPC_PORT`, `LDK_ALICE_GRPC_PORT` and
`BITCOIND_ZMQ_PORT`. Their defaults are in
[nix/local-env.nix](nix/local-env.nix).

The keys the environment runs on are fixed in [.env](.env). They belong to this
regtest network alone. It also pins `DATA_SYNC_VERSION`, the commit of
[data-sync](https://github.com/breez/data-sync) both setups build.

## Layout

| Path | What it is |
|---|---|
| [docker-compose.yml](docker-compose.yml) | Every service, its image and its ports |
| [scripts/](scripts) | What sets the environment up and keeps it running |
| [tools.dockerfile](tools.dockerfile) | The image those scripts run in, with the SSP's and the nodes' CLIs |
| [config/](config) | What a service reads unchanged |
| [nix/](nix) | The same environment, built and run by Nix |

The operator, the SSP and the Lightning nodes are built from the dockerfiles in
`crates/spark-itest/docker/`, at the commits pinned there. The integration tests
build their images from the same directory, except for the operator's, which
come from the `-private` dockerfiles.
