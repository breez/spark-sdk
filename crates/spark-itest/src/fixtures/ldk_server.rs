use std::fs;
use std::time::Duration;

use anyhow::{Context, Result};
use bip39::Mnemonic;
use bitcoin::Network;
use bitcoin::bip32::{ChildNumber, Xpriv};
use bitcoin::secp256k1::{Secp256k1, SecretKey};
use testcontainers::core::wait::LogWaitStrategy;
use testcontainers::core::{ContainerPort, ExecCommand, Mount, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};
use tokio::time::sleep;
use tracing::{info, warn};

use crate::fixtures::bitcoind::BitcoindFixture;
use crate::fixtures::log::TracingConsumer;
use crate::fixtures::setup::FixtureId;

pub const GRPC_PORT: u16 = 3536;
const LIGHTNING_PORT: u16 = 9735;

pub struct LdkServerFixture {
    pub container: ContainerAsync<GenericImage>,
    pub container_name: String,
    /// Host address of the gRPC service: `127.0.0.1:<mapped port>`.
    pub base_url: String,
    pub api_key: String,
    pub cert_pem: Vec<u8>,
    /// `None` when the node's entropy is not on disk where the fixture looks.
    pub node_secret_key: Option<SecretKey>,
}

impl LdkServerFixture {
    pub async fn start(
        fixture_id: &FixtureId,
        bitcoind: &BitcoindFixture,
        name: &str,
    ) -> Result<Self> {
        let config_dir = fixture_id.testdir()?;
        let config_path = config_dir.join(format!("{name}.toml"));
        let container_name = format!("ldk-{name}-{fixture_id}");

        let config = format!(
            r#"[node]
network = "regtest"
listening_addresses = ["0.0.0.0:{LIGHTNING_PORT}"]
announcement_addresses = ["{container_name}:{LIGHTNING_PORT}"]
alias = "{name}"

[storage.disk]
dir_path = "/data"

[log]
level = "Info"

[bitcoind]
rpc_address = "{}"
rpc_user = "{}"
rpc_password = "{}"
"#,
            bitcoind.internal_rpc_url, bitcoind.rpcuser, bitcoind.rpcpassword
        );
        fs::write(&config_path, config)?;

        // ldk-server's own certificate names only localhost and 127.0.0.1, but a
        // client in another container reaches it by container name.
        let (tls_key_pem, tls_cert_pem) =
            crate::fixtures::spark_so::generate_self_signed_certificate(&[
                container_name.clone(),
                "localhost".to_string(),
                "127.0.0.1".to_string(),
            ])?;
        let tls_cert_path = config_dir.join(format!("{name}-tls.crt"));
        let tls_key_path = config_dir.join(format!("{name}-tls.key"));
        fs::write(&tls_cert_path, &tls_cert_pem)?;
        fs::write(&tls_key_path, &tls_key_pem)?;

        let container = GenericImage::new("ldk-server", "latest")
            .with_exposed_port(ContainerPort::Tcp(GRPC_PORT))
            .with_exposed_port(ContainerPort::Tcp(LIGHTNING_PORT))
            .with_wait_for(WaitFor::Log(LogWaitStrategy::stdout(
                "gRPC service listening on",
            )))
            .with_network(fixture_id.to_network())
            .with_container_name(&container_name)
            .with_log_consumer(TracingConsumer::new(format!("ldk-{name}")))
            .with_mount(Mount::bind_mount(
                config_path.display().to_string(),
                "/config/ldk-server.toml",
            ))
            .with_mount(Mount::bind_mount(
                tls_cert_path.display().to_string(),
                "/data/tls.crt",
            ))
            .with_mount(Mount::bind_mount(
                tls_key_path.display().to_string(),
                "/data/tls.key",
            ))
            .with_cmd(["/config/ldk-server.toml"])
            .start()
            .await?;

        let host_grpc_port = crate::fixtures::published_port(&container, GRPC_PORT).await?;
        let base_url = format!("127.0.0.1:{host_grpc_port}");

        // ldk-server stores the API key as 32 raw bytes; the HMAC key is their
        // lowercase hex encoding.
        let api_key_bytes = read_in_container(&container, "/data/regtest/api_key").await?;
        let api_key = hex::encode(api_key_bytes);
        let cert_pem = read_in_container(&container, "/data/tls.crt").await?;
        let node_secret_key = read_node_secret_key(&container).await?;

        info!("ldk-server '{name}' ready at {base_url} (peer {container_name}:{LIGHTNING_PORT})");
        Ok(Self {
            container,
            container_name,
            base_url,
            api_key,
            cert_pem,
            node_secret_key,
        })
    }

    pub fn peer_address(&self) -> String {
        format!("{}:{LIGHTNING_PORT}", self.container_name)
    }

    pub fn node_secret_key_hex(&self) -> Option<String> {
        self.node_secret_key
            .map(|key| hex::encode(key.secret_bytes()))
    }
}

/// Searched for by name, since ldk-server namespaces some storage files by
/// network and not others. Missing entropy only costs re-signing invoices as
/// this node, so it is not an error.
async fn read_node_secret_key(
    container: &ContainerAsync<GenericImage>,
) -> Result<Option<SecretKey>> {
    let found = try_exec(container, &["find", "/data", "-name", "keys_mnemonic"]).await?;
    let Some(path) = found
        .lines()
        .next()
        .map(str::trim)
        .filter(|p| !p.is_empty())
    else {
        let listing = try_exec(container, &["find", "/data", "-type", "f"]).await?;
        warn!(
            "ldk-server keeps no entropy this knows how to read, so invoices cannot be re-signed \
             as this node. It holds: {}",
            listing.split_whitespace().collect::<Vec<_>>().join(" ")
        );
        return Ok(None);
    };
    let words = read_in_container(container, path).await?;
    let mnemonic = Mnemonic::parse(std::str::from_utf8(&words)?.trim())
        .context("parsing ldk-server's recovery phrase")?;
    // ldk-server gives ldk-node no passphrase, and ldk-node then stretches the
    // phrase with an empty one.
    Ok(Some(derive_node_secret_key(&mnemonic.to_seed(""))?))
}

async fn try_exec(container: &ContainerAsync<GenericImage>, argv: &[&str]) -> Result<String> {
    let mut result = container
        .exec(ExecCommand::new(argv.iter().copied()))
        .await?;
    Ok(String::from_utf8_lossy(&result.stdout_to_vec().await?).into_owned())
}

/// Mirrors ldk-node, which seeds LDK's `KeysManager` with the private key of a
/// BIP32 master over its seed. The node key is hardened child 0 of the master
/// `KeysManager` builds from that.
fn derive_node_secret_key(seed: &[u8]) -> Result<SecretKey> {
    let secp = Secp256k1::new();
    let node_xprv = Xpriv::new_master(Network::Regtest, seed)
        .context("deriving the ldk-node master key from its seed")?;
    // `KeysManager` derives with testnet on every network, and the network does
    // not change the key.
    let ldk_master = Xpriv::new_master(Network::Testnet, &node_xprv.private_key.secret_bytes())
        .context("deriving the ldk keys-manager master key")?;
    Ok(ldk_master
        .derive_priv(&secp, &ChildNumber::Hardened { index: 0 })?
        .private_key)
}

async fn read_in_container(
    container: &ContainerAsync<GenericImage>,
    path: &str,
) -> Result<Vec<u8>> {
    for _ in 0..40 {
        let mut result = container.exec(ExecCommand::new(["cat", path])).await?;
        let bytes = result.stdout_to_vec().await?;
        if !bytes.is_empty() {
            return Ok(bytes);
        }
        sleep(Duration::from_millis(250)).await;
    }
    anyhow::bail!("file {path} not present in ldk-server container after retries")
}
