fn main() {
    println!("cargo:rerun-if-changed=src/postgresql/migrations");

    tonic_build::configure()
        .build_server(false)
        .build_client(true)
        .extern_path(".spark", "::spark::operator::rpc::spark")
        .extern_path(".common", "::spark::operator::rpc::common")
        .compile_protos(
            &[
                "proto/spark/spark_ssp_internal.proto",
                "proto/spark/spark_internal.proto",
            ],
            &["proto", "../../spark/protos"],
        )
        .unwrap();
}
