use std::{process::Stdio, time::Duration};

use tempfile::TempDir;
use tokio::{io::AsyncWriteExt, process::Command, time::timeout};

const SPARK_CONFIG: &str = r#"{
    "coordinator_identifier": "0000000000000000000000000000000000000000000000000000000000000001",
    "threshold": 1,
    "signing_operators": [{
        "id": 0,
        "identifier": "0000000000000000000000000000000000000000000000000000000000000001",
        "address": "https://signet-operator.invalid",
        "identity_public_key": "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
    }],
    "ssp_config": {
        "base_url": "https://signet-ssp.invalid",
        "identity_public_key": "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
    },
    "expected_withdraw_bond_sats": 10000,
    "expected_withdraw_relative_block_locktime": 1000
}"#;

fn cli(data_dir: &TempDir) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cli"));
    command
        .args(["--network", "signet", "--server-mode", "--data-dir"])
        .arg(data_dir.path())
        .env_remove("BREEZ_API_KEY")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    command
}

#[tokio::test]
async fn signet_starts_with_explicit_services() {
    let data_dir = TempDir::new().unwrap();
    let config_path = data_dir.path().join("spark.json");
    std::fs::write(&config_path, SPARK_CONFIG).unwrap();
    let mut child = cli(&data_dir)
        .arg("--spark-config")
        .arg(&config_path)
        .args(["--chain-api-url", "https://signet-chain.invalid/api"])
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"exit\n")
        .await
        .unwrap();
    let output = timeout(Duration::from_secs(30), child.wait_with_output())
        .await
        .expect("CLI should start without contacting Signet services")
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Breez SDK CLI Interactive Mode"));
    assert!(stdout.contains("Goodbye!"));
}

#[tokio::test]
async fn signet_requires_explicit_chain_service() {
    let data_dir = TempDir::new().unwrap();
    let config_path = data_dir.path().join("spark.json");
    std::fs::write(&config_path, SPARK_CONFIG).unwrap();
    let output = timeout(
        Duration::from_secs(30),
        cli(&data_dir)
            .arg("--spark-config")
            .arg(&config_path)
            .output(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("Signet requires an explicit chain service")
    );
}

#[tokio::test]
async fn signet_requires_explicit_spark_config() {
    let data_dir = TempDir::new().unwrap();
    let output = timeout(Duration::from_secs(30), cli(&data_dir).output())
        .await
        .unwrap()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("Signet requires an explicit spark_config")
    );
}

#[tokio::test]
async fn reports_unreadable_and_invalid_spark_config() {
    let data_dir = TempDir::new().unwrap();
    let config_path = data_dir.path().join("spark.json");
    for (contents, expected) in [
        (None, "Failed to open Spark config"),
        (Some("{"), "Failed to parse Spark config"),
        (Some("{}"), "missing field"),
    ] {
        if let Some(contents) = contents {
            std::fs::write(&config_path, contents).unwrap();
        }
        let output = timeout(
            Duration::from_secs(30),
            cli(&data_dir)
                .arg("--spark-config")
                .arg(&config_path)
                .output(),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(!output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(expected), "{stderr}");
        assert!(stderr.contains(config_path.to_str().unwrap()), "{stderr}");
    }
}
