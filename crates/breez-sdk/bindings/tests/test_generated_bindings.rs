use std::process::Command;

#[test]
fn test_golang() {
    let output = Command::new("go")
        .env(
            "CGO_LDFLAGS",
            "-lbreez_sdk_spark_bindings -L../../../ffi/golang -Wl,-rpath,../../../ffi/golang",
        )
        .env("CGO_ENABLED", "1")
        .current_dir("tests/bindings/golang/")
        .arg("run")
        .arg("./")
        .output()
        .expect("failed to execute process");
    println!("status: {}", output.status);
    println!("stdout: {}", String::from_utf8_lossy(&output.stdout));
    println!("stderr: {}", String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success());
}
