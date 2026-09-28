//! Prints the cache key for the itest bootstrap snapshot.

use anyhow::Result;
use spark_itest::fixtures::state_snapshot;

fn main() -> Result<()> {
    println!("{}", state_snapshot::bootstrap_key()?);
    Ok(())
}
