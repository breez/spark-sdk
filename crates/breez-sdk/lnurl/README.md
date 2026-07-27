# LNURL Server

This crate provides an LNURL server implementation for Breez SDK - Spark. 

## About LNURL

LNURL is a protocol for Lightning Network interactions such as payments, withdrawals, and authentication. This LNURL server implements:

- **LNURL-pay**: Allows receiving Lightning payments via URLs or QR codes
- **Lightning Address**: Enables email-like addresses for receiving payments (username@domain.com)

A user can register their username together with their Spark public key with the server. The server will then:

1. Serve LNURL endpoints on the user's behalf
2. Create invoices on the user's behalf without the user being online
3. Handle Lightning Address lookups at `/.well-known/lnurlp/username`

> **Trust Model**: The user needs to trust that the LNURL server and the SSP (Spark Service Provider) do not collude by sharing the preimage. Additionally, the user must trust the LNURL server as it could return invoices that are not directed to the user at all.

## Prerequisites

To compile and run the LNURL server, you'll need:

- Rust toolchain (1.75 or newer recommended)
- Protobuf compiler (`protoc`)
- OpenSSL development libraries
- PostgreSQL

## How to Compile

### Installing Dependencies

#### On Debian/Ubuntu:
```shell
apt-get update
apt-get install -y libprotobuf-dev libssl-dev pkg-config protobuf-compiler
```

#### On macOS (with Homebrew):
```shell
brew install protobuf openssl pkg-config
```

### Building the Server

From the repository root:

```shell
cargo build --release --manifest-path crates/breez-sdk/lnurl/Cargo.toml
```

The compiled binary will be available at `target/release/lnurl`.

## How to Run

### Docker (Recommended for Production)

Building the Docker image:

```shell
docker build -t lnurl-server -f crates/breez-sdk/lnurl/Dockerfile .
```

Running the container:

```shell
docker run -p 8080:8080 \
  -e BREEZ_LNURL_DB_URL="postgres://user:password@postgres_host:5432/lnurl_db" \
  -e BREEZ_LNURL_DOMAINS="yourdomain.com" \
  -e BREEZ_LNURL_DEFAULT_API_KEY="<breez-api-key>" \
  -e BREEZ_LNURL_AUTO_MIGRATE=true \
  lnurl-server
```

### Native (Rust)

If you've built the binary, you can run it directly:

```shell
./target/release/lnurl --db-url="postgres://user:password@localhost:5432/lnurl_db" --domains="yourdomain.com" --default-api-key="<breez-api-key>" --auto-migrate
```

## Configuration

The server can be configured in three ways (highest precedence first):

1. Command-line arguments
2. Environment variables (prefixed with `BREEZ_LNURL_`)
3. Config file (TOML format)

Only flags actually passed on the command line take precedence; a flag left at
its default does not override a value set via environment variable or config file.

### Configuration File

Create a file named `lnurl.conf` (or specify a different path with `--config`):

```toml
# Server configuration
address = "0.0.0.0:8080"
auto_migrate = true
log_level = "info"
network = "mainnet"
scheme = "https"                    # Scheme for generated URLs only; the server
                                    # binds plain HTTP, terminate TLS at a proxy

# Database configuration
db_url = "postgres://user:password@localhost:5432/lnurl_db"

# LNURL payment configuration
min_sendable = 1000                 # Minimum amount in millisatoshi (1 sat)
max_sendable = 4000000000           # Maximum amount in millisatoshi (4,000,000 sats)
domains = "yourdomain.com"          # Comma-separated list of allowed domains
default_api_key = "<breez-api-key>" # Fallback Breez API key for partner attribution (required on mainnet)
```

### Important Configuration Options

| Option | Description | Default |
|--------|-------------|---------|
| `--address` | Address the server listens on | `0.0.0.0:8080` |
| `--auto-migrate` | Automatically apply database migrations | `false` |
| `--db-url` | PostgreSQL connection string | `""` |
| `--domains` | Comma-separated list of allowed domains | `localhost:8080` |
| `--default-api-key` | Fallback Breez API key for partner attribution (**required on mainnet**) | (none) |
| `--log-level` | RUST_LOG style format (e.g., `info`, `lnurl=trace,info`, `lnurl=trace,spark_wallet=debug,info`) | `info` |
| `--network` | Spark network (mainnet, testnet, regtest) | `mainnet` |
| `--min-sendable` | Minimum payment amount (millisatoshi) | `1000` |
| `--max-sendable` | Maximum payment amount (millisatoshi) | `4000000000` |
| `--webhook-domain` | Domain for the webhook URL registered with the SSP | (none) |
| `--ssp-auth-seed` | Hex-encoded 32-byte seed for SSP authentication | (random) |

The allowed domains are the ones stored in the database, which the server refreshes
periodically and to which `--domains` is added on startup. If that list ends up empty
(`--domains=""` and no domains in the database), requests for any host are accepted on
testnet and regtest, which is intended for local and test setups only. On mainnet an
empty list rejects every request instead of falling open.

For a complete list of options, run:
```shell
lnurl --help
```

### Partner Attribution

This server creates invoices on behalf of its users, so each lightning-address receive is attributed to a partner when the invoice is created.

A receive is attributed to the domain's own Breez API key when one is configured for that domain, otherwise to the **default API key** (`--default-api-key` / `BREEZ_LNURL_DEFAULT_API_KEY`).

On **mainnet the default API key is required**, and the server will not start without it. This ensures a self-hosted server attributes all of its receives (to its own partner) instead of leaving any unattributed.

### Database Support

PostgreSQL is the only supported backend. `--db-url` must start with `postgres://`
or the server refuses to start.

When `--auto-migrate` is enabled, the server will automatically create the required tables.

## Server API Endpoints

The LNURL server provides the following endpoints:

### Public Endpoints

- `/.well-known/lnurlp/{username}` - LNURL-pay endpoint for Lightning Address handling
- `/lnurlp/{username}` - Alternative LNURL-pay endpoint 
- `/lnurlp/{username}/invoice` - Invoice generation endpoint for LNURL-pay

### Authenticated Endpoints (require API key)

- `/lnurlpay/available/{username}` - Check if a username is available
- `/lnurlpay/{pubkey}` - Register a username (POST) or unregister (DELETE)
- `/lnurlpay/{pubkey}/recover` - Recover a username registration

## Example Usage

### Setting Up For Development

```shell
# Point at a local database with auto-migrations
./target/release/lnurl --db-url="postgres://user:password@localhost:5432/lnurl_db" \
  --domains="localhost:8080" \
  --auto-migrate \
  --scheme="http"
```

### Setting Up For Production

```shell
# Setup PostgreSQL database with auto-migrations
./target/release/lnurl --db-url="postgres://user:password@localhost:5432/lnurl_db" \
  --domains="yourdomain.com" \
  --auto-migrate \
  --address="0.0.0.0:8080"
```

### Docker Compose Example

```yaml
version: '3'

services:
  postgres:
    image: postgres:15
    environment:
      POSTGRES_USER: lnurl
      POSTGRES_PASSWORD: password
      POSTGRES_DB: lnurl_db
    volumes:
      - postgres_data:/var/lib/postgresql/data
    restart: unless-stopped

  lnurl-server:
    build:
      context: .
      dockerfile: crates/breez-sdk/lnurl/Dockerfile
    environment:
      BREEZ_LNURL_DB_URL: "postgres://lnurl:password@postgres:5432/lnurl_db"
      BREEZ_LNURL_DOMAINS: "yourdomain.com"
      BREEZ_LNURL_AUTO_MIGRATE: "true"
      BREEZ_LNURL_ADDRESS: "0.0.0.0:8080"
      BREEZ_LNURL_DEFAULT_API_KEY: "<breez-api-key>"
    ports:
      - "8080:8080"
    depends_on:
      - postgres
    restart: unless-stopped

volumes:
  postgres_data:
```

## Testing

The tests run against a real PostgreSQL instance. Each test gets its own schema,
so they can share one database, but the tests create and drop schemas in it:
point `LNURL_TEST_POSTGRES_URL` at a disposable instance, never at real data.

```shell
docker run -d --rm --name lnurl-pg-test \
  -e POSTGRES_PASSWORD=postgres -e POSTGRES_DB=lnurl_test \
  -p 55432:5432 postgres:16-alpine

LNURL_TEST_POSTGRES_URL="postgres://postgres:postgres@localhost:55432/lnurl_test" \
  make lnurl-test
```

Without `LNURL_TEST_POSTGRES_URL` the database-backed tests fail rather than
silently skip.

## License

See LICENSE in the repository root.
