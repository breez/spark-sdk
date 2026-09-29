fn main() {
    tonic_build::configure()
        .emit_rerun_if_changed(true)
        .build_client(true)
        .build_server(false)
        .compile_protos(
            &["../spark-service-provider/sspd/proto/ssp_internal/ssp_internal.proto"],
            &["../spark-service-provider/sspd/proto/ssp_internal"],
        )
        .unwrap();
}
