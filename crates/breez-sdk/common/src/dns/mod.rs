#[cfg_attr(
    all(target_family = "wasm", target_os = "unknown"),
    path = "resolver_wasm.rs"
)]
mod resolver;

use anyhow::Result;
use dnssec_prover::rr::{Name, RR};
use dnssec_prover::ser::parse_rr_stream;
use dnssec_prover::validation::verify_rr_stream;
use web_time::{SystemTime, UNIX_EPOCH};

pub use resolver::Resolver;

#[macros::async_trait]
pub trait DnsResolver {
    async fn txt_lookup(&self, dns_name: String) -> Result<Vec<String>>;
}

/// Normalizes a DNS name to FQDN format (with trailing dot) as required by dnssec-prover.
fn normalize_dns_name(dns_name: String) -> String {
    if dns_name.ends_with('.') {
        dns_name
    } else {
        dns_name + "."
    }
}

/// Parses a DNS name string into a dnssec-prover Name.
fn parse_dns_name(dns_name: &str) -> Result<Name> {
    Name::try_from(dns_name).map_err(|()| anyhow::anyhow!("Invalid DNS name: {}", dns_name))
}

/// Verifies a DNSSEC proof and extracts TXT records for the given name.
///
/// This function handles:
/// - Parsing the proof into resource records
/// - Verifying the DNSSEC chain
/// - Checking time validity
/// - Resolving CNAME chains
/// - Extracting TXT records
fn verify_proof_and_extract_txt(proof: &[u8], name: &Name) -> Result<Vec<String>> {
    // Parse the proof into resource records
    let rrs = parse_rr_stream(proof).map_err(|()| anyhow::anyhow!("Failed to parse DNS proof"))?;

    // Verify the DNSSEC chain
    let verified = verify_rr_stream(&rrs)
        .map_err(|e| anyhow::anyhow!("DNSSEC verification failed: {:?}", e))?;

    // Check that the proof is currently valid
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    if now < verified.valid_from || now > verified.expires {
        anyhow::bail!(
            "DNSSEC proof is not currently valid (valid from {} to {}, current time {})",
            verified.valid_from,
            verified.expires,
            now
        );
    }

    // Resolve the name to get the correct records (handles CNAME chains)
    let resolved_rrs = verified.resolve_name(name);

    // Extract TXT records from resolved records
    let txt_records: Vec<String> = resolved_rrs
        .into_iter()
        .filter_map(|rr| {
            if let RR::Txt(txt) = rr {
                Some(String::from_utf8_lossy(&txt.data.as_vec()).into_owned())
            } else {
                None
            }
        })
        .collect();

    Ok(txt_records)
}
