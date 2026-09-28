//! Prints `<image>=<tag>` for each image named, or for every image a local
//! cluster runs when none is, for `cargo xtask` to build them under and for CI to
//! cache them by.

use anyhow::Result;

fn main() -> Result<()> {
    let named: Vec<String> = std::env::args().skip(1).collect();
    let images: Vec<&str> = if named.is_empty() {
        spark_itest::images::ALL.to_vec()
    } else {
        named.iter().map(String::as_str).collect()
    };
    for image in images {
        println!("{image}={}", spark_itest::images::tag(image)?);
    }
    Ok(())
}
