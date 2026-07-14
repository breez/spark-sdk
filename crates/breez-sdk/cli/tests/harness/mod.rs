//! Harness for the shared behavioral scenarios in `tests/scenarios/`.
//!
//! The scenario JSON files are shared verbatim with the language CLI ports
//! (each port has its own runner); this module is the Rust runner. See
//! `tests/scenarios/README.md` for the schema and the sync contract.

pub mod assert;
pub mod engine;
pub mod faucet;
pub mod session;

use std::collections::HashMap;

use anyhow::{Result, bail};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Scenario {
    pub name: String,
    /// Runtime requirements: "faucet" (regtest faucet credentials) and/or
    /// "docker". The runner skips the scenario when one is unmet.
    #[serde(default)]
    pub requires: Vec<String>,
    /// Fixtures the runner provisions before the first session. "lnurl"
    /// exposes `${lnurl_url}`.
    #[serde(default)]
    pub fixtures: Vec<String>,
    pub sessions: Vec<Session>,
}

#[derive(Debug, Deserialize)]
pub struct Session {
    /// Wallet key: sessions sharing a key share a data dir (same mnemonic).
    pub wallet: String,
    #[serde(default)]
    pub extra_args: Vec<String>,
    pub steps: Vec<Step>,
}

/// One scenario step: either a REPL command (`cmd`) or a harness action
/// (`faucet_fund`), never both.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub cmd: Option<String>,
    /// Scripted answers for the command's interactive prompts, written to
    /// stdin in order right after the command line.
    #[serde(default)]
    pub stdin: Vec<String>,
    /// JSON-path assertions against the last JSON document the step printed.
    #[serde(default)]
    pub expect_json: serde_json::Map<String, serde_json::Value>,
    /// Substring assertions against the step's raw transcript chunk.
    #[serde(default)]
    pub expect_contains: Vec<String>,
    /// Variables to capture from the last JSON document: name to JSON path.
    #[serde(default)]
    pub capture: HashMap<String, String>,
    pub retry: Option<Retry>,
    pub faucet_fund: Option<FaucetFund>,
}

#[derive(Debug, Deserialize)]
pub struct Retry {
    pub timeout_secs: u64,
    #[serde(default = "default_retry_interval")]
    pub interval_secs: u64,
}

fn default_retry_interval() -> u64 {
    5
}

#[derive(Debug, Deserialize)]
pub struct FaucetFund {
    pub address: String,
    pub amount_sats: u64,
}

/// Replace every `${name}` in `input` with its value from `vars`. Unknown
/// variables are an error so scenario typos fail loudly.
pub fn interpolate(input: &str, vars: &HashMap<String, String>) -> Result<String> {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("${") {
        let Some(end_rel) = rest[start..].find('}') else {
            bail!("unterminated ${{...}} in '{input}'");
        };
        let end = start.checked_add(end_rel).expect("index overflow");
        out.push_str(&rest[..start]);
        let name = &rest[start.saturating_add(2)..end];
        match vars.get(name) {
            Some(value) => out.push_str(value),
            None => bail!("unknown variable '${{{name}}}' in '{input}'"),
        }
        rest = &rest[end.saturating_add(1)..];
    }
    out.push_str(rest);
    Ok(out)
}
