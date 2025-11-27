fn main() {
    let target_family =
        std::env::var("CARGO_CFG_TARGET_FAMILY").expect("CARGO_CFG_TARGET_FAMILY not set");
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").expect("CARGO_CFG_TARGET_OS not set");
    let is_wasm = target_family == "wasm" && target_os == "unknown";

    tonic_prost_build::configure()
        .build_server(false)
        .build_client(true)
        .build_transport(!is_wasm)
        .compile_protos(
            &[
                "protos/spark/common.proto",
                "protos/spark/spark.proto",
                "protos/spark/spark_authn.proto",
                "protos/spark/spark_token.proto",
            ],
            &["protos"],
        )
        .unwrap();

    println!("cargo:rerun-if-changed=protos");

    built::write_built_file().expect("Failed to acquire build-time information");
}
