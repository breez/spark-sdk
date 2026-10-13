use std::fmt::Write as _;
use std::path::{Path, PathBuf};

// Embeds every source file for the models test that checks nothing serde
// buffers holds a u128 or a tagged enum. A test running in wasm can't list
// the directory itself.
fn main() {
    let src = Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("src");
    println!("cargo:rerun-if-changed={}", src.display());

    let mut files = Vec::new();
    collect_rust_files(&src, &mut files);
    files.sort();

    let mut out = String::from("&[\n");
    for file in files {
        writeln!(out, "    include_str!({:?}),", file.display().to_string()).unwrap();
    }
    out.push(']');

    let dest = Path::new(&std::env::var("OUT_DIR").unwrap()).join("wasm_sources.rs");
    std::fs::write(dest, out).unwrap();
}

fn collect_rust_files(dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect_rust_files(&path, files);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
        }
    }
}
