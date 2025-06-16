fn main() {
    tonic_build::configure()
        .build_server(false)
        .type_attribute(".", "#[derive(serde::Serialize, serde::Deserialize)]")
        .type_attribute(".", "#[serde(rename_all = \"camelCase\")]")
        .compile_protos(
            &[
                "protos/spark/common.proto",
                "protos/spark/spark.proto",
                "protos/spark/spark_tree.proto",
                "protos/spark/frost.proto",
                "protos/spark/spark_authn.proto",
            ],
            &["protos"],
        )
        .unwrap();

    println!("cargo:rerun-if-changed=spark-protos");
}
