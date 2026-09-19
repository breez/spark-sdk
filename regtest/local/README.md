# Local Spark environment

A Spark regtest network of your own: bitcoind, three Spark operators, a Spark
service provider (SSP) with its Lightning node, an LNURL server, a data-sync
service, and the mempool explorer with its API. Documented for integrators in
[Testing and development](https://sdk-doc-spark.breez.technology/guide/testing.html).

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

## Settings

Every setting is an environment variable read when the environment starts:
`BITCOIND_RPC_PORT=18500 docker compose -f regtest/local/docker-compose.yml up`.

| Setting | Default | What it sets |
|---|---|---|
| `BITCOIND_RPC_PORT` | `18443` | Bitcoin Core's RPC port |
| `BITCOIND_P2P_PORT` | `18444` | Bitcoin Core's P2P port |
| `OPERATOR_0_PORT` to `OPERATOR_2_PORT` | `8535` to `8537` | The operators' ports |
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
| `TLS_EXTRA_HOSTS` | unset | More names or addresses the operators' certificate covers |
| `BLOCK_INTERVAL_SECONDS` | `5` | Seconds between mined blocks. `0` mines the first 200 and no more |
| `CHANNEL_SATS` | `5000000000` | The channel Alice opens with the SSP's node, half of it pushed |
| `LEAVES_PER_DENOMINATION` | `8` | Leaves the SSP keeps of each denomination |
| `MAX_DENOMINATION_POWER` | `16` | Largest denomination the SSP keeps, in powers of two sats |
| `DKG_MIN_AVAILABLE_KEYS` | `12000` | Keyshares each operator keeps unused. The SSP's pool spends over 10,000 |
| `READY_TIMEOUT_SECONDS` | `2400` | How long the environment reports progress before it gives up |
| `SPARK_LOCAL_DIR` | `./.spark-local` | Where Nix keeps the environment's state |

Nix reads a few more, for the ports its services hold to one host:
`POSTGRES_PORT`, `ELECTRS_PORT`, `ELECTRS_ELECTRUM_PORT`,
`ELECTRS_MONITORING_PORT`, `MEMPOOL_API_PORT`, `LDK_GRPC_PORT`,
`LDK_ALICE_GRPC_PORT` and `BITCOIND_ZMQ_PORT`. Their defaults are in
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
`crates/spark-itest/docker/`, which the integration tests build their own images
from, at the commits pinned there.
