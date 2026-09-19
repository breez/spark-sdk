# Testing and development

There are three networks to test an integration against:

- The [**Regtest Network**](#regtest-network) maintained by Lightspark, for most testing and development
- A [**local environment**](#local-environment): a Spark regtest network of your own, running on your machine
- [**Mainnet with small amounts**](#mainnet-testing), for features that depend on live networks

## Regtest Network

For most testing and development, we recommend using the **Regtest Network** - a deployed test network maintained by Lightspark that is free to use and carries no real-world value.

### What you can test on Regtest

- **Spark Payments**: Bitcoin and token payments using the Spark protocol
- **Deposits**: Receiving test Bitcoin from the [Lightspark Regtest Faucet](https://app.lightspark.com/regtest-faucet)
- **Withdrawals**: Sending funds back to on-chain addresses
- **Token Issuance**: Creating and testing tokens using the SDK's [issuing functionality](./issuing_tokens.md)

### Getting started

1. [Initialize the SDK](./initializing.md) using the default regtest config (no API key required)
2. [Generate a Bitcoin receiving address](./receive_payment.md#bitcoin)
3. Request funds from the [faucet](https://app.lightspark.com/regtest-faucet) to your generated address
4. Test all Spark-related functionality in a controlled development environment

## Local environment

The local environment is a complete Spark network on your own machine: a Bitcoin Core node in regtest mode, three Spark operators, a Spark service provider (SSP) with its Lightning node, a second Lightning node the SSP's holds a channel with, an LNURL server, a data-sync service, and a [mempool](https://mempool.space) block explorer with its API. It shares nothing with any other network, so the chain moves only when the environment mines, funds come from the environment's own Bitcoin node, and a reset returns everything to an empty chain.

### What you can test locally

- **Spark payments**: Bitcoin and token payments between wallets on the environment
- **Deposits**: On-chain deposits, funded from the environment's Bitcoin node
- **Withdrawals**: Sending funds back to on-chain addresses
- **Lightning payments**, between wallets on the environment and with Alice, the second Lightning node
- **Lightning addresses**: registered with, and served by, the environment's own LNURL server
- **Multi-device sync**: a wallet's data kept in step across its instances by the environment's data-sync service
- **Token issuance**, using the SDK's [issuing functionality](./issuing_tokens.md)
- **Unilateral exits**, which need the chain mined past a [timelock](./unilateral_exit.md)

Alice stands in for the Lightning network outside the environment: she and the SSP's node share a 50 BTC channel, funded on both sides, so a wallet can pay her and be paid by her. A payment to any other node fails, since the environment is not connected to one.

### Starting the environment

The environment runs on macOS and Linux, either in Docker or natively with Nix. The first start builds the Spark operator, the SSP and the Lightning node from source, which takes a while. A new environment then needs a few more minutes before it can serve wallets: the operators generate the signing keys the SSP needs to build its pool of leaves.

**Docker**: from a clone of the [spark-sdk repository](https://github.com/breez/spark-sdk), run:

```bash
docker compose -f regtest/local/docker-compose.yml up
```

It builds what it needs and starts every service in order, reporting what it waits on as it goes: the operators generating their signing keys, then the SSP stocking its leaf pool. It ends by printing the addresses below. `make local-env-up` does the same in the background, `make local-env-down` stops it and keeps its state, and `make local-env-reset` deletes it.

**Nix**: with flakes enabled, run:

```bash
nix run github:breez/spark-sdk#local-env
```

The environment runs in the foreground until you quit it, and keeps its state in `./.spark-local`, or in `SPARK_LOCAL_DIR` when set. Deleting that directory resets it.

Docker writes the environment's Spark configuration to `regtest/local/data/spark-config.json`, and Nix to `spark-config.json` in its state directory. Both print where it is, along with these endpoints:

| Endpoint | Address | Port setting |
|---|---|---|
| Chain API (mempool.space) | `http://127.0.0.1:8090/api` | `MEMPOOL_PORT` |
| Mempool explorer | `http://127.0.0.1:8090` | `MEMPOOL_PORT` |
| Spark service provider (SSP) | `http://127.0.0.1:59049` | `SSP_PORT` |
| LNURL server | `http://127.0.0.1:8080` | `LNURL_PORT` |
| Data-sync service | `http://127.0.0.1:8081` | `DATA_SYNC_PORT` |
| Data-sync service, for browsers | `http://127.0.0.1:8082` | `DATA_SYNC_WEB_PORT` |
| Bitcoin Core RPC (`rpcuser` / `rpcpassword`) | `http://127.0.0.1:18443` | `BITCOIND_RPC_PORT` |

Every port is an environment variable, so an address already in use can be moved: `BITCOIND_RPC_PORT=18500 docker compose -f regtest/local/docker-compose.yml up`. The ports, what the environment mines and what the SSP keeps are listed in the environment's [README](https://github.com/breez/spark-sdk/blob/main/regtest/local/README.md).

### Connecting the SDK

Start from the default config for {{#enum Network::Regtest}} and change five things:

- **Spark environment**: {{#name parse_spark_config}} reads the configuration file the environment writes, which carries its operators, its SSP and the certificate authority their TLS certificates are issued by. Set the result on {{#name spark_config}}. The fields are those of the [Spark environment configuration](./config.md#spark-environment-configuration).
- **Chain service**: use a [REST chain service](./customizing.md#with-rest-chain-service) at `http://127.0.0.1:8090/api`, of type {{#enum ChainApiType::MempoolSpace}}.
- **Deposit claim fee**: the environment's SSP can quote more to claim a deposit than the default {{#name max_deposit_claim_fee}} of 1 sat/vbyte allows. Deposits it quotes above the ceiling wait to be [claimed manually](./onchain_claims.md#manually-claiming-deposits), unless the ceiling is raised.
- **Lightning address domain**: set {{#name lnurl_domain}} to the environment's LNURL server, `http://127.0.0.1:8080`, which [registering an address](./receive_lnurl_pay.md) then goes to.
- **Sync server**: set {{#name real_time_sync_server_url}} to the environment's data-sync service, `http://127.0.0.1:8081`, which keeps [a wallet's instances in step](./config.md#real-time-sync-server-url). In a browser, use its gRPC-Web address, `http://127.0.0.1:8082`.

{{#tabs config:local-spark-config}}

Both setups print a command for each service, including `ldk-server-cli` calls that invoice from Alice and pay a wallet's invoice with her.

### Funding wallets and mining blocks

The environment mines a block every 5 seconds. To send a wallet funds from the environment's Bitcoin node, or to mine blocks at once, for example to pass a timelock:

| | Docker | Nix |
|---|---|---|
| Fund an address | `make local-env-fund ADDRESS=<address> AMOUNT_SATS=<sats>` | `nix run github:breez/spark-sdk#local-env -- fund <address> <sats>` |
| Mine blocks | `make local-env-mine BLOCKS=<blocks>` | `nix run github:breez/spark-sdk#local-env -- mine <blocks>` |

### Testing from another device

By default every service accepts connections only from the machine the environment runs on, and the operators' certificates cover `localhost`, `127.0.0.1` and `10.0.2.2`, the address under which the Android emulator reaches its host. To test from a phone on your network, start a stopped environment with `BIND_ADDRESS=0.0.0.0` and `PUBLIC_HOST` set to your machine's address on that network, for example `BIND_ADDRESS=0.0.0.0 PUBLIC_HOST=192.168.1.10 make local-env-up`. The Spark configuration file then points wallets at `PUBLIC_HOST`, and the operators' certificates cover it.

## Mainnet testing

Some features rely on live networks that Regtest doesn't reproduce. Test these on **Mainnet with small amounts**: use real satoshis, but keep transaction values very low while verifying the flows work correctly.

### Lightning payments

The Regtest Network doesn't have a developed Lightning Network, so test Lightning send and receive flows on Mainnet.

### Stable balance and USDC/USDT

The stablecoin assets are only available on Mainnet:

- **USDB** is the Spark-native stablecoin behind [Stable Balance](./stable_balance.md).
- **USDC** and **USDT** are cross-chain assets. Use [USDC/USDT](./cross_chain.md) to pay recipients on their native chains or receive from them. The cross-chain providers operate against live external networks and have no testnet equivalent.

Test these integrations on Mainnet with small amounts.

## Development best practices

- **Start with Regtest** for most development and testing
- **Use a local environment** to control the chain, or to test without depending on a shared network
- **Use Mainnet** for Lightning, stable balance, and USDC/USDT testing
- **Test all payment types** you plan to support in your application
