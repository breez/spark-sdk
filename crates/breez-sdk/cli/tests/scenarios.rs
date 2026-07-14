//! Runs the shared behavioral scenarios in `tests/scenarios/` against the
//! Rust CLI binary. One test per scenario file, so failures are greppable by
//! scenario name. The whole suite soft-skips unless `FAUCET_USERNAME` is set
//! (see `harness::engine::run_scenario`).

mod harness;

use anyhow::Result;

#[tokio::test]
async fn scenario_01_get_info_sync() -> Result<()> {
    harness::engine::run_scenario("01_get_info_sync").await
}

#[tokio::test]
async fn scenario_02_receive_and_parse() -> Result<()> {
    harness::engine::run_scenario("02_receive_and_parse").await
}

#[tokio::test]
async fn scenario_03_deposit_claim_and_spark_pay() -> Result<()> {
    harness::engine::run_scenario("03_deposit_claim_and_spark_pay").await
}
