//! Prints `<image>=<tag>` for every image a local cluster runs, for `cargo xtask`
//! to build them under and for CI to cache them by.

use anyhow::Result;

fn main() -> Result<()> {
    for image in spark_itest::images::ALL {
        println!("{image}={}", spark_itest::images::tag(image)?);
    }
    Ok(())
}
