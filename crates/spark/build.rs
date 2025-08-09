fn main() {
    tonic_build::configure()
        .build_server(false)
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
}
