use std::io::Write as _;
use std::process::Command;
use std::{env, str::FromStr};

use anyhow::{Context, Result, bail};

use crate::package::{TargetPackage, WasmPackages, package_cmd};

#[derive(Debug, Clone)]
pub enum DocSnippetsPackage {
    Wasm,
    Flutter,
    Go,
    KotlinMPP,
    Python,
    ReactNative,
    Rust,
    Swift,
    CSharp,
}

impl FromStr for DocSnippetsPackage {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "wasm" => Ok(DocSnippetsPackage::Wasm),
            "flutter" => Ok(DocSnippetsPackage::Flutter),
            "go" => Ok(DocSnippetsPackage::Go),
            "kotlin-mpp" => Ok(DocSnippetsPackage::KotlinMPP),
            "python" => Ok(DocSnippetsPackage::Python),
            "react-native" => Ok(DocSnippetsPackage::ReactNative),
            "rust" => Ok(DocSnippetsPackage::Rust),
            "swift" => Ok(DocSnippetsPackage::Swift),
            "csharp" => Ok(DocSnippetsPackage::CSharp),
            _ => bail!("invalid target package: {}", s),
        }
    }
}

pub fn check_doc_snippets_cmd(
    package: Option<DocSnippetsPackage>,
    skip_binding_gen: bool,
) -> Result<()> {
    match package {
        Some(DocSnippetsPackage::Wasm) => {
            check_doc_snippets_wasm_cmd(skip_binding_gen)?;
        }
        Some(DocSnippetsPackage::Flutter) => {
            check_doc_snippets_flutter_cmd(skip_binding_gen)?;
        }
        Some(DocSnippetsPackage::Go) => {
            check_doc_snippets_go_cmd(skip_binding_gen)?;
        }
        Some(DocSnippetsPackage::KotlinMPP) => {
            check_doc_snippets_kotlin_mpp_cmd(skip_binding_gen)?;
        }
        Some(DocSnippetsPackage::Python) => {
            check_doc_snippets_python_cmd(skip_binding_gen)?;
        }
        Some(DocSnippetsPackage::ReactNative) => {
            check_doc_snippets_react_native_cmd(skip_binding_gen)?;
        }
        Some(DocSnippetsPackage::Rust) => {
            check_doc_snippets_rust_cmd()?;
        }
        Some(DocSnippetsPackage::Swift) => {
            check_doc_snippets_swift_cmd(skip_binding_gen)?;
        }
        Some(DocSnippetsPackage::CSharp) => {
            check_doc_snippets_csharp_cmd(skip_binding_gen)?;
        }
        None => {
            check_doc_snippets_wasm_cmd(skip_binding_gen)?;
            check_doc_snippets_flutter_cmd(skip_binding_gen)?;
            check_doc_snippets_go_cmd(skip_binding_gen)?;
            check_doc_snippets_kotlin_mpp_cmd(skip_binding_gen)?;
            check_doc_snippets_python_cmd(skip_binding_gen)?;
            check_doc_snippets_react_native_cmd(skip_binding_gen)?;
            check_doc_snippets_rust_cmd()?;
            check_doc_snippets_swift_cmd(skip_binding_gen)?;
            check_doc_snippets_csharp_cmd(skip_binding_gen)?;
        }
    }
    Ok(())
}

fn check_doc_snippets_wasm_cmd(skip_binding_gen: bool) -> Result<()> {
    let workspace_root = env::current_dir()?;
    let doc_snippets_dir = workspace_root.join("docs/breez-sdk/snippets/wasm");

    if !skip_binding_gen {
        println!("Creating WASM package");
        package_cmd(Some(TargetPackage::Wasm(WasmPackages::All)))?;

        // Run yarn cache clean
        let status = Command::new("yarn")
            .arg("cache")
            .arg("clean")
            .current_dir(&doc_snippets_dir)
            .status()?;
        if !status.success() {
            anyhow::bail!("Doc snippet check failed: `yarn cache clean` failed");
        }

        // Remove node_modules directory if it exists
        let node_modules_dir = doc_snippets_dir.join("node_modules");
        if node_modules_dir.exists() {
            println!("Removing node_modules");
            std::fs::remove_dir_all(&node_modules_dir)?;
        }
        // Remove yarn.lock file if it exists
        let yarn_lock_file = doc_snippets_dir.join("yarn.lock");
        if yarn_lock_file.exists() {
            println!("Removing yarn.lock");
            std::fs::remove_file(&yarn_lock_file)?;
        }
    }

    println!("Checking doc snippets WASM");

    // Run yarn install (yarn)
    let status = Command::new("yarn")
        .current_dir(&doc_snippets_dir)
        .status()?;
    if !status.success() {
        anyhow::bail!("Doc snippet check failed: `yarn` failed");
    }

    // Run tsc
    let status = Command::new("tsc")
        .current_dir(&doc_snippets_dir)
        .status()?;
    if !status.success() {
        anyhow::bail!("Doc snippet check failed: `tsc` failed");
    }

    // Run yarn run lint
    let status = Command::new("yarn")
        .arg("run")
        .arg("lint")
        .current_dir(&doc_snippets_dir)
        .status()?;
    if !status.success() {
        anyhow::bail!("Doc snippet check failed: `yarn run lint` failed");
    }

    Ok(())
}

fn check_doc_snippets_flutter_cmd(skip_binding_gen: bool) -> Result<()> {
    let workspace_root = env::current_dir()?;

    if !skip_binding_gen {
        println!("Generating Flutter bindings");

        let flutter_package_dir = workspace_root.join("packages/flutter");
        let status = Command::new("make")
            .arg("generate-bindings")
            .current_dir(&flutter_package_dir)
            .status()?;
        if !status.success() {
            anyhow::bail!("Failed to generate Flutter bindings: `make generate-bindings` failed");
        }
    }

    println!("Checking doc snippets Flutter");

    let doc_snippets_flutter_dir = workspace_root.join("docs/breez-sdk/snippets/flutter");

    // Run flutter pub get
    let status = Command::new("flutter")
        .arg("pub")
        .arg("get")
        .current_dir(&doc_snippets_flutter_dir)
        .status()?;
    if !status.success() {
        anyhow::bail!("Doc snippet check failed: `flutter pub get` failed");
    }

    // Run dart analyze --fatal-infos
    let status = Command::new("dart")
        .arg("analyze")
        .arg("--fatal-infos")
        .current_dir(&doc_snippets_flutter_dir)
        .status()?;
    if !status.success() {
        anyhow::bail!("Doc snippet check failed: `dart analyze --fatal-infos` failed");
    }

    Ok(())
}

fn check_doc_snippets_go_cmd(skip_binding_gen: bool) -> Result<()> {
    let workspace_root = env::current_dir()?;

    if !skip_binding_gen {
        println!("Building Go package");

        let bindings_dir = workspace_root.join("crates/breez-sdk/bindings");
        let status = Command::new("make")
            .arg("bindings-golang")
            .current_dir(&bindings_dir)
            .status()?;
        if !status.success() {
            anyhow::bail!("Failed to generate golang bindings: `make bindings-golang` failed");
        }

        // Copy golang directory to packages and rename to breez-sdk-spark-go
        let source_dir = workspace_root.join("crates/breez-sdk/bindings/ffi/golang");
        let dest_dir =
            workspace_root.join("docs/breez-sdk/snippets/go/packages/breez-sdk-spark-go");
        let packages_dir = workspace_root.join("docs/breez-sdk/snippets/go/packages");

        // Create packages directory if it doesn't exist
        if !packages_dir.exists() {
            std::fs::create_dir_all(&packages_dir)?;
        }

        // Remove destination if it exists to ensure clean copy
        if dest_dir.exists() {
            std::fs::remove_dir_all(&dest_dir)?;
        }

        let status = Command::new("cp")
            .arg("-r")
            .arg(&source_dir)
            .arg(&dest_dir)
            .status()?;
        if !status.success() {
            anyhow::bail!(
                "Failed to copy golang bindings from {:?} to {:?}",
                source_dir,
                dest_dir
            );
        }

        // Create go.mod file with the required contents at the destination directory
        let go_mod_path = dest_dir.join("go.mod");
        let go_mod_contents = r#"module breez-sdk-spark-go

go 1.19

"#;

        let mut file = std::fs::File::create(&go_mod_path)?;
        file.write_all(go_mod_contents.as_bytes())?;
    }

    println!("Checking doc snippets Go");

    let go_snippets_dir = workspace_root.join("docs/breez-sdk/snippets/go");
    let status = Command::new("go")
        .arg("get")
        .current_dir(&go_snippets_dir)
        .status()?;
    if !status.success() {
        anyhow::bail!("Failed to run 'go get' in {:?}", go_snippets_dir);
    }

    let status = Command::new("go")
        .arg("build")
        .arg(".")
        .current_dir(&go_snippets_dir)
        .status()?;
    if !status.success() {
        anyhow::bail!("Failed to run 'go build .' in {:?}", go_snippets_dir);
    }

    Ok(())
}

fn check_doc_snippets_kotlin_mpp_cmd(skip_binding_gen: bool) -> Result<()> {
    let workspace_root = env::current_dir()?;

    if !skip_binding_gen {
        println!("Building Kotlin MPP Bindings");

        let bindings_dir = workspace_root.join("crates/breez-sdk/bindings");
        let status = Command::new("make")
            .arg("package-kotlin-multiplatform-dummy-binaries")
            .current_dir(&bindings_dir)
            .status()?;
        if !status.success() {
            anyhow::bail!(
                "Failed to run 'make package-kotlin-multiplatform-dummy-binaries' in {:?}",
                bindings_dir
            );
        }

        let kotlin_mpp_dir = bindings_dir.join("langs/kotlin-multiplatform");
        let status = Command::new("./gradlew")
            .arg("publishToMavenLocal")
            .arg("-PlibraryVersion=0.0.0-local-docs")
            .current_dir(&kotlin_mpp_dir)
            .status()?;
        if !status.success() {
            anyhow::bail!(
                "Failed to run 'gradlew publishToMavenLocal' in {:?}",
                kotlin_mpp_dir
            );
        }
    }

    println!("Checking doc snippets Kotlin MPP");

    let kotlin_snippets_dir = workspace_root.join("docs/breez-sdk/snippets/kotlin_mpp_lib");
    let status = Command::new("./gradlew")
        .arg("build")
        .current_dir(&kotlin_snippets_dir)
        .status()?;
    if !status.success() {
        anyhow::bail!(
            "Failed to run './gradlew build' in {:?}",
            kotlin_snippets_dir
        );
    }

    Ok(())
}

fn check_doc_snippets_python_cmd(skip_binding_gen: bool) -> Result<()> {
    let workspace_root = env::current_dir()?;

    if !skip_binding_gen {
        println!("Building Python package");

        let bindings_dir = workspace_root.join("crates/breez-sdk/bindings");
        let status = Command::new("make")
            .arg("bindings-python")
            .current_dir(&bindings_dir)
            .status()?;
        if !status.success() {
            anyhow::bail!("Failed to run 'make bindings-python' in {:?}", bindings_dir);
        }

        // Copy of files from ffi/python to langs/python/src/breez_sdk_spark
        let src_dir = bindings_dir.join("ffi/python");
        let dst_dir = bindings_dir.join("langs/python/src/breez_sdk_spark");

        for entry in std::fs::read_dir(&src_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() {
                let file_name = path.file_name().unwrap();
                std::fs::copy(&path, dst_dir.join(file_name))?;
            }
        }

        // Create py.typed marker file so mypy recognizes the package as typed
        let py_typed_path = dst_dir.join("py.typed");
        std::fs::File::create(&py_typed_path)?;
    }

    println!("Checking doc snippets Python");

    let python_snippets_dir = workspace_root.join("docs/breez-sdk/snippets/python");
    let python_pkg_dir = workspace_root.join("crates/breez-sdk/bindings/langs/python");

    let venv_dir = "/tmp/breez_sdk_python_venv";
    let venv_pip = format!("{}/bin/pip", venv_dir);

    // Create venv
    let status = Command::new("python3")
        .arg("-m")
        .arg("venv")
        .arg(venv_dir)
        .status()?;
    if !status.success() {
        anyhow::bail!("Failed to create venv at {}", venv_dir);
    }

    // Upgrade pip in venv
    let status = Command::new(&venv_pip)
        .arg("install")
        .arg("--upgrade")
        .arg("pip")
        .status()?;
    if !status.success() {
        anyhow::bail!("Failed to upgrade pip in venv");
    }

    // Install the package from langs/python
    let status = Command::new(&venv_pip)
        .arg("install")
        .arg(".")
        .current_dir(&python_pkg_dir)
        .status()?;
    if !status.success() {
        anyhow::bail!("Failed to install Python package in venv");
    }

    // Install pylint
    let status = Command::new(&venv_pip)
        .arg("install")
        .arg("pylint")
        .status()?;
    if !status.success() {
        anyhow::bail!("Failed to install pylint in venv");
    }

    // Install mypy for type checking
    let status = Command::new(&venv_pip)
        .arg("install")
        .arg("mypy")
        .status()?;
    if !status.success() {
        anyhow::bail!("Failed to install mypy in venv");
    }

    // Run mypy for type checking
    // Using minimal flags to catch SDK method misuse without requiring full type annotations
    let venv_mypy = format!("{}/bin/mypy", venv_dir);
    let status = Command::new(&venv_mypy)
        .arg("src")
        .arg("--disable-error-code=misc")
        .arg("--disable-error-code=no-untyped-def")
        .arg("--disable-error-code=no-untyped-call")
        .arg("--disable-error-code=import-untyped")
        .arg("--disable-error-code=no-any-return")
        .arg("--disable-error-code=arg-type")
        .arg("--no-warn-no-return")
        .arg("--allow-untyped-defs")
        .current_dir(&python_snippets_dir)
        .status()?;
    if !status.success() {
        anyhow::bail!(
            "mypy type checks failed in {:?}",
            python_snippets_dir.join("src")
        );
    }

    // Run pylint in python_snippets_dir on src/
    let venv_pylint = format!("{}/bin/pylint", venv_dir);
    let status = Command::new(&venv_pylint)
        .arg("-d")
        .arg("W0612,W0622,W1203,R0801,R0903,C0114,C0115,C0116,R1702")
        .arg("src")
        .current_dir(&python_snippets_dir)
        .status()?;
    if !status.success() {
        anyhow::bail!(
            "pylint checks failed in {:?}",
            python_snippets_dir.join("src")
        );
    }

    Ok(())
}

fn check_doc_snippets_react_native_cmd(skip_binding_gen: bool) -> Result<()> {
    let workspace_root = env::current_dir()?;
    let doc_snippets_dir = workspace_root.join("docs/breez-sdk/snippets/react-native");

    if !skip_binding_gen {
        println!("Building React Native bindings");

        let react_native_pkg_dir = workspace_root.join("packages/react-native");

        // Remove node_modules in packages/react-native if it exists
        let node_modules_dir = react_native_pkg_dir.join("node_modules");
        if node_modules_dir.exists() {
            std::fs::remove_dir_all(&node_modules_dir)
                .with_context(|| format!("Failed to remove {:?}", node_modules_dir))?;
        }

        // Run `yarn --mode=skip-build`
        let status = Command::new("yarn")
            .arg("--mode=skip-build")
            .current_dir(&react_native_pkg_dir)
            .status()?;
        if !status.success() {
            anyhow::bail!("`yarn --mode=skip-build` failed in packages/react-native");
        }

        // Run `npx patch-package`
        let status = Command::new("npx")
            .arg("patch-package")
            .current_dir(&react_native_pkg_dir)
            .status()?;
        if !status.success() {
            anyhow::bail!("`npx patch-package` failed in packages/react-native");
        }

        // TODO: Skip building binaries (right now building just for ios as it's enough to generate the bindings)
        // Run `yarn ubrn:ios`
        let status = Command::new("yarn")
            .arg("ubrn:ios")
            .current_dir(&react_native_pkg_dir)
            .status()?;
        if !status.success() {
            anyhow::bail!("`yarn ubrn:ios` failed in packages/react-native");
        }

        // Run `yarn prepare`
        let status = Command::new("yarn")
            .arg("prepare")
            .current_dir(&react_native_pkg_dir)
            .status()?;
        if !status.success() {
            anyhow::bail!("`yarn prepare` failed in packages/react-native");
        }

        // Remove node_modules in docs/breez-sdk/snippets/react-native if it exists
        let node_modules_dir = doc_snippets_dir.join("node_modules");
        if node_modules_dir.exists() {
            std::fs::remove_dir_all(&node_modules_dir)
                .with_context(|| format!("Failed to remove {:?}", node_modules_dir))?;
        }
    }

    println!("Checking doc snippets React Native");

    // Run yarn install (yarn)
    let status = Command::new("yarn")
        .env("EXPO_PUBLIC_SKIP_POSTINSTALL", "1")
        .current_dir(&doc_snippets_dir)
        .status()?;
    if !status.success() {
        anyhow::bail!("Doc snippet check failed: `yarn` failed");
    }

    // Run tsc
    let status = Command::new("yarn")
        .arg("tsc")
        .current_dir(&doc_snippets_dir)
        .status()?;
    if !status.success() {
        anyhow::bail!("Doc snippet check failed: `yarn tsc` failed");
    }

    // Run yarn run lint
    let status = Command::new("yarn")
        .arg("run")
        .arg("lint")
        .current_dir(&doc_snippets_dir)
        .status()?;
    if !status.success() {
        anyhow::bail!("Doc snippet check failed: `yarn run lint` failed");
    }

    Ok(())
}

fn check_doc_snippets_rust_cmd() -> Result<()> {
    println!("Checking doc snippets Rust");

    let workspace_root = env::current_dir()?;
    let doc_snippets_dir = workspace_root.join("docs/breez-sdk/snippets/rust");

    // Run cargo clippy with the requested arguments
    let status = Command::new("cargo")
        .arg("clippy")
        .arg("--")
        .arg("--allow")
        .arg("dead_code")
        .arg("--allow")
        .arg("unused_variables")
        .arg("--deny")
        .arg("warnings")
        .current_dir(&doc_snippets_dir)
        .status()?;
    if !status.success() {
        anyhow::bail!("Doc snippet check failed: `cargo clippy` failed");
    }

    Ok(())
}

fn check_doc_snippets_csharp_cmd(skip_binding_gen: bool) -> Result<()> {
    let workspace_root = env::current_dir()?;

    if !skip_binding_gen {
        println!("Building C# package with dummy binaries");

        let bindings_dir = workspace_root.join("crates/breez-sdk/bindings");

        // Generate C# bindings
        let status = Command::new("make")
            .arg("bindings-csharp")
            .current_dir(&bindings_dir)
            .status()?;
        if !status.success() {
            anyhow::bail!("Failed to generate C# bindings: `make bindings-csharp` failed");
        }

        // Create dummy binaries for all platforms
        let csharp_src_dir = bindings_dir.join("langs/csharp/src");
        let runtimes = vec![
            ("osx-arm64", "libbreez_sdk_spark_bindings.dylib"),
            ("osx-x64", "libbreez_sdk_spark_bindings.dylib"),
            ("linux-arm64", "libbreez_sdk_spark_bindings.so"),
            ("linux-x64", "libbreez_sdk_spark_bindings.so"),
            ("win-x64", "breez_sdk_spark_bindings.dll"),
            ("win-x86", "breez_sdk_spark_bindings.dll"),
        ];

        for (platform, lib_name) in runtimes {
            let runtime_dir = csharp_src_dir.join(format!("runtimes/{}/native", platform));
            std::fs::create_dir_all(&runtime_dir)?;
            let lib_path = runtime_dir.join(lib_name);
            // Create empty dummy file
            std::fs::File::create(&lib_path)?;
        }

        // Delete existing .cs files in the src directory (except the project file)
        for entry in std::fs::read_dir(&csharp_src_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("cs") {
                std::fs::remove_file(&path)?;
            }
        }

        // Copy only the main C# binding file to the src directory
        let ffi_csharp_file = bindings_dir.join("ffi/csharp/breez_sdk_spark.cs");
        std::fs::copy(&ffi_csharp_file, csharp_src_dir.join("breez_sdk_spark.cs"))?;

        // Pack the NuGet package
        let status = Command::new("dotnet")
            .arg("pack")
            .arg("-c")
            .arg("Release")
            .arg("-p:Version=0.0.0-local-docs")
            .current_dir(&csharp_src_dir)
            .status()?;
        if !status.success() {
            anyhow::bail!("Failed to pack NuGet package: `dotnet pack` failed");
        }

        // Add local NuGet source if not already added
        let nuget_source = csharp_src_dir.join("bin/Release");
        let _ = Command::new("dotnet")
            .arg("nuget")
            .arg("add")
            .arg("source")
            .arg(&nuget_source)
            .arg("-n")
            .arg("LocalBreezSdkSpark")
            .status();
    }

    println!("Checking doc snippets C#");

    let csharp_snippets_dir = workspace_root.join("docs/breez-sdk/snippets/csharp");

    // Clear NuGet cache for our local package to ensure latest version is used
    let _ = Command::new("dotnet")
        .arg("nuget")
        .arg("locals")
        .arg("all")
        .arg("--clear")
        .status();

    // Restore NuGet packages
    let status = Command::new("dotnet")
        .arg("restore")
        .current_dir(&csharp_snippets_dir)
        .status()?;
    if !status.success() {
        anyhow::bail!("Doc snippet check failed: `dotnet restore` failed");
    }

    // Build the project
    let status = Command::new("dotnet")
        .arg("build")
        .arg("--no-restore")
        .current_dir(&csharp_snippets_dir)
        .status()?;
    if !status.success() {
        anyhow::bail!("Doc snippet check failed: `dotnet build` failed");
    }

    // Format check
    let status = Command::new("dotnet")
        .arg("format")
        .arg("--verify-no-changes")
        .current_dir(&csharp_snippets_dir)
        .status()?;
    if !status.success() {
        anyhow::bail!("Doc snippet check failed: `dotnet format --verify-no-changes` failed");
    }

    Ok(())
}

fn check_doc_snippets_swift_cmd(skip_binding_gen: bool) -> Result<()> {
    // Ensure we are running on macOS, because Swift doc snippet checking only works there
    if !cfg!(target_os = "macos") {
        anyhow::bail!("Swift doc snippet checks must be run on macOS.");
    }

    let workspace_root = env::current_dir()?;

    if !skip_binding_gen {
        println!("Building Swift package");

        let bindings_dir = workspace_root.join("crates/breez-sdk/bindings");

        // Generate Swift bindings and framework structure (this also builds the release binary via dependency)
        let status = Command::new("make")
            .arg("package-xcframework-no-binaries")
            .current_dir(&bindings_dir)
            .status()?;
        if !status.success() {
            anyhow::bail!(
                "Doc snippet check failed: `make package-xcframework-no-binaries` failed"
            );
        }

        // Copy the host architecture binary into the macOS framework location
        let target_dir = workspace_root.join("target");
        let host_binary = target_dir.join("release/libbreez_sdk_spark_bindings.a");
        let framework_binary = bindings_dir.join("langs/swift/breez_sdk_sparkFFI.xcframework/macos-arm64_x86_64/breez_sdk_sparkFFI.framework/breez_sdk_sparkFFI");

        std::fs::copy(&host_binary, &framework_binary).with_context(|| {
            format!(
                "Failed to copy binary from {:?} to {:?}",
                host_binary, framework_binary
            )
        })?;
    }

    println!("Checking doc snippets Swift");

    let snippets_swift_dir = workspace_root.join("docs/breez-sdk/snippets/swift/BreezSdkSnippets");

    let status = Command::new("swift")
        .arg("package")
        .arg("clean")
        .current_dir(&snippets_swift_dir)
        .status()?;
    if !status.success() {
        anyhow::bail!("Doc snippet check failed: `swift package clean` failed");
    }

    let status = Command::new("swift")
        .arg("build")
        .current_dir(&snippets_swift_dir)
        .status()?;
    if !status.success() {
        anyhow::bail!("Doc snippet check failed: `swift build` failed");
    }

    let status = Command::new("swift")
        .arg("run")
        .current_dir(&snippets_swift_dir)
        .status()?;
    if !status.success() {
        anyhow::bail!("Doc snippet check failed: `swift run` failed");
    }

    Ok(())
}
