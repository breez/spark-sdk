fn main() {
    println!("cargo:rerun-if-changed=src/postgresql/migrations");

    tonic_build::configure()
        .emit_rerun_if_changed(true)
        .build_server(true)
        .compile_protos(
            &["proto/ssp_internal/ssp_internal.proto"],
            &["proto/ssp_internal"],
        )
        .unwrap();

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

    // No generated client: a tonic interceptor cannot read the request body, which
    // the HMAC in ldk-server's `x-auth` header covers.
    tonic_build::configure()
        .build_server(false)
        .build_client(false)
        .compile_protos(&["proto/ldk_server/api.proto"], &["proto/ldk_server"])
        .unwrap();
    println!("cargo:rerun-if-changed=proto/ldk_server");

    tonic_build::configure()
        .build_server(false)
        .build_client(false)
        .compile_protos(&["proto/ssp_authn/ssp_authn.proto"], &["proto/ssp_authn"])
        .unwrap();
    println!("cargo:rerun-if-changed=proto/ssp_authn");
}
