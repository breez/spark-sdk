//! Loads a scenario file and runs its sessions against the CLI binary.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use super::session::{CliLaunch, CliSession};
use super::{FaucetFund, Scenario, Step, assert, faucet, interpolate};

/// Marker-wait ceiling for a single command; commands that need longer
/// (claims, syncs) wrap in `retry` at the scenario level.
const STEP_TIMEOUT: Duration = Duration::from_mins(2);

/// Run a scenario by file stem, skipping (Ok) when its requirements are not
/// met. All scenarios need network access to the remote regtest operators,
/// so `FAUCET_USERNAME` doubles as the opt-in flag for the whole suite.
pub async fn run_scenario(name: &str) -> Result<()> {
    if std::env::var("FAUCET_USERNAME").is_err() {
        eprintln!("skipping scenario {name}: FAUCET_USERNAME not set");
        return Ok(());
    }

    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/scenarios")
        .join(format!("{name}.json"));
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let scenario: Scenario =
        serde_json::from_str(&raw).with_context(|| format!("invalid scenario {name}"))?;

    if scenario.requires.iter().any(|r| r == "docker") && !docker_available() {
        eprintln!("skipping scenario {name}: docker not available");
        return Ok(());
    }

    let mut vars: HashMap<String, String> = HashMap::new();
    let _fixtures = start_fixtures(&scenario, &mut vars).await?;

    let launch = CliLaunch::from_env()?;
    let mut wallet_dirs: HashMap<String, tempfile::TempDir> = HashMap::new();
    for (session_index, session) in scenario.sessions.iter().enumerate() {
        let dir = match wallet_dirs.entry(session.wallet.clone()) {
            std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
            std::collections::hash_map::Entry::Vacant(e) => {
                e.insert(tempfile::tempdir().context("failed to create wallet dir")?)
            }
        };
        let extra_args = session
            .extra_args
            .iter()
            .map(|a| interpolate(a, &vars))
            .collect::<Result<Vec<_>>>()?;

        let mut cli = CliSession::spawn(&launch, dir.path(), &extra_args)?;
        for (step_index, step) in session.steps.iter().enumerate() {
            run_step(&mut cli, step, &mut vars).await.with_context(|| {
                format!(
                    "scenario '{}', session {session_index} (wallet '{}'), step {step_index}",
                    scenario.name, session.wallet
                )
            })?;
        }
        cli.close().await?;
    }
    Ok(())
}

async fn run_step(
    cli: &mut CliSession,
    step: &Step,
    vars: &mut HashMap<String, String>,
) -> Result<()> {
    match (&step.cmd, &step.faucet_fund) {
        (None, Some(fund)) => run_faucet_fund(fund, vars).await,
        (Some(cmd), None) => run_cmd_step(cli, cmd, step, vars).await,
        _ => bail!("a step must have exactly one of 'cmd' or 'faucet_fund'"),
    }
}

async fn run_faucet_fund(fund: &FaucetFund, vars: &HashMap<String, String>) -> Result<()> {
    let address = interpolate(&fund.address, vars)?;
    let txid = faucet::fund_address(&address, fund.amount_sats).await?;
    eprintln!(
        "faucet funded {address} with {} sats: {txid}",
        fund.amount_sats
    );
    Ok(())
}

async fn run_cmd_step(
    cli: &mut CliSession,
    cmd: &str,
    step: &Step,
    vars: &mut HashMap<String, String>,
) -> Result<()> {
    let cmd = interpolate(cmd, vars)?;
    let stdin = step
        .stdin
        .iter()
        .map(|l| interpolate(l, vars))
        .collect::<Result<Vec<_>>>()?;

    let deadline = step.retry.as_ref().map(|r| {
        std::time::Instant::now()
            .checked_add(Duration::from_secs(r.timeout_secs))
            .expect("deadline overflow")
    });
    let interval = Duration::from_secs(step.retry.as_ref().map_or(0, |r| r.interval_secs));

    loop {
        let chunk = cli.run_step(&cmd, &stdin, STEP_TIMEOUT).await?;
        match evaluate(&chunk, step, vars) {
            Ok(captured) => {
                vars.extend(captured);
                return Ok(());
            }
            Err(e) => match deadline {
                Some(deadline) if std::time::Instant::now() < deadline => {
                    eprintln!("step '{cmd}' not satisfied yet ({e}), retrying");
                    tokio::time::sleep(interval).await;
                }
                _ => {
                    return Err(e.context(format!("step '{cmd}' failed; step output:\n{chunk}")));
                }
            },
        }
    }
}

/// Check a step's expectations against its transcript chunk; on success
/// return the variables it captures.
fn evaluate(
    chunk: &str,
    step: &Step,
    vars: &HashMap<String, String>,
) -> Result<HashMap<String, String>> {
    let chunk = &assert::strip_prompts(chunk);
    let docs = assert::extract_json_docs(chunk);
    let last = docs.last();

    for (path, matcher) in &step.expect_json {
        let matcher = interpolate_value(matcher, vars)?;
        let found = last.and_then(|doc| assert::lookup_path(doc, path));
        assert::check_matcher(&matcher, found)
            .with_context(|| format!("expect_json '{path}' failed"))?;
    }

    for needle in &step.expect_contains {
        let needle = interpolate(needle, vars)?;
        if !chunk.contains(&needle) {
            bail!("expect_contains '{needle}' not found in step output");
        }
    }

    let mut captured = HashMap::new();
    for (name, path) in &step.capture {
        let value = last
            .and_then(|doc| assert::lookup_path(doc, path))
            .with_context(|| format!("capture '{name}': path '{path}' not found"))?;
        captured.insert(name.clone(), assert::value_to_string(value));
    }
    Ok(captured)
}

/// Interpolate `${var}` inside a matcher's string forms.
fn interpolate_value(matcher: &Value, vars: &HashMap<String, String>) -> Result<Value> {
    Ok(match matcher {
        Value::String(s) => Value::String(interpolate(s, vars)?),
        other => other.clone(),
    })
}

fn docker_available() -> bool {
    std::process::Command::new("docker")
        .arg("version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Fixture handles that clean up on drop. "lnurl" exposes `${lnurl_url}`.
async fn start_fixtures(
    scenario: &Scenario,
    vars: &mut HashMap<String, String>,
) -> Result<Vec<super::lnurl::LnurlFixture>> {
    let mut fixtures = Vec::new();
    for fixture in &scenario.fixtures {
        if fixture == "lnurl" {
            let lnurl = super::lnurl::LnurlFixture::start().await?;
            vars.insert("lnurl_url".to_string(), lnurl.http_url.clone());
            fixtures.push(lnurl);
        } else {
            bail!("unknown fixture '{fixture}'");
        }
    }
    Ok(fixtures)
}
