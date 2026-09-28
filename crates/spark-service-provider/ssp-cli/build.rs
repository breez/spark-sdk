fn main() {
    tonic_build::configure()
        .emit_rerun_if_changed(true)
        .build_client(true)
        .build_server(false)
        .type_attribute(".", "#[derive(serde::Serialize, serde::Deserialize)]")
        .compile_protos(
            &["../sspd/proto/ssp_internal/ssp_internal.proto"],
            &["../sspd/proto/ssp_internal"],
        )
        .unwrap();
}
