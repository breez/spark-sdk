use std::process::Command;

fn main() {
    let target_family =
        std::env::var("CARGO_CFG_TARGET_FAMILY").expect("CARGO_CFG_TARGET_FAMILY not set");
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").expect("CARGO_CFG_TARGET_OS not set");
    let is_wasm = target_family == "wasm" && target_os == "unknown";

    tonic_build::configure()
        .build_server(false)
        .build_client(true)
        .build_transport(!is_wasm)
        .compile_protos(
            &[
                "protos/spark/common.proto",
                "protos/spark/multisig.proto",
                "protos/spark/spark.proto",
                "protos/spark/spark_authn.proto",
                "protos/spark/spark_token.proto",
                "protos/ssp/ssp_authn.proto",
            ],
            &["protos"],
        )
        .unwrap();

    println!("cargo:rerun-if-changed=protos");

    if let Some(version) = git_version() {
        println!("cargo:rustc-env=SPARK_GIT_VERSION={version}");
    }
}

fn git_version() -> Option<String> {
    // Find the repository from this directory: git sets GIT_DIR for the hooks
    // of a linked worktree, and that can be another repository, such as the
    // repository of an app that depends on this crate.
    let output = Command::new("git")
        .args(["describe", "--tags", "--always", "--abbrev=7"])
        .env_remove("GIT_DIR")
        .output()
        .ok()?;
    let version = String::from_utf8(output.stdout).ok()?;
    let version = version.trim();
    (output.status.success() && !version.is_empty()).then(|| version.to_string())
}
