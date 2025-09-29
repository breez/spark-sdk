use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    str::FromStr,
};

use anyhow::{Context as _, Result, bail};
use xshell::{Shell, cmd};

use crate::detect_brew_llvm_paths;

#[derive(Debug, Clone)]
pub enum TargetPackage {
    Wasm(WasmPackages),
}

impl FromStr for TargetPackage {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.to_lowercase();
        let split = s.split("::").collect::<Vec<&str>>();
        if split.is_empty() || split.len() > 2 {
            bail!(
                "invalid target package: {} - expected format: <package>[::<subpackage>]",
                s
            );
        }
        match split[0] {
            "wasm" => {
                let wasm_package = if split.len() == 1 {
                    // No subpackage specified, default to All
                    WasmPackages::All
                } else {
                    // Subpackage specified, parse it
                    WasmPackages::from_str(split[1])?
                };
                Ok(TargetPackage::Wasm(wasm_package))
            }
            _ => bail!("invalid target package: {}", s),
        }
    }
}

#[derive(Debug, Clone)]
pub enum WasmPackages {
    All,
    Node,
    Deno,
    Web,
    Bundle,
}

impl FromStr for WasmPackages {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "all" => Ok(WasmPackages::All),
            "node" => Ok(WasmPackages::Node),
            "deno" => Ok(WasmPackages::Deno),
            "web" => Ok(WasmPackages::Web),
            "bundle" => Ok(WasmPackages::Bundle),
            _ => bail!("invalid wasm package: {}", s),
        }
    }
}

pub fn package_cmd(package: Option<TargetPackage>) -> Result<()> {
    match package {
        Some(TargetPackage::Wasm(wasm_package)) => {
            package_wasm_cmd(wasm_package)?;
        }
        None => {
            println!("No package specified, packaging all packages");
            package_wasm_cmd(WasmPackages::All)?;
        }
    }
    Ok(())
}

fn package_wasm_cmd(wasm_package: WasmPackages) -> Result<()> {
    let sh = Shell::new()?;

    // Ensure wasm-pack exists
    let _ = cmd!(sh, "cargo install wasm-pack --locked").run();

    // Get workspace root and set up paths
    let workspace_root = std::env::current_dir()?;
    let wasm_crate_dir = workspace_root.join("crates/breez-sdk/wasm");
    let pkg_dir = workspace_root.join("packages/wasm");

    // On macOS, auto-detect Homebrew LLVM and set CC/AR for wasm cross-compiles
    let clang_env = if cfg!(target_os = "macos")
        && let Some((clang_path, llvm_ar_path)) = detect_brew_llvm_paths(&sh)
    {
        vec![
            ("CC_wasm32_unknown_unknown".to_string(), clang_path),
            ("AR_wasm32_unknown_unknown".to_string(), llvm_ar_path),
        ]
    } else {
        vec![]
    };

    println!("Packaging WASM target: {:?}", wasm_package);

    match wasm_package {
        WasmPackages::All => {
            println!("Packaging all WASM targets");
            package_wasm_target(&wasm_crate_dir, &pkg_dir, "bundler", &clang_env)?;
            package_wasm_target(&wasm_crate_dir, &pkg_dir, "deno", &clang_env)?;
            package_wasm_target(&wasm_crate_dir, &pkg_dir, "nodejs", &clang_env)?;
            package_wasm_target(&wasm_crate_dir, &pkg_dir, "web", &clang_env)?;
        }
        WasmPackages::Bundle => {
            println!("Packaging Bundle WASM target");
            package_wasm_target(&wasm_crate_dir, &pkg_dir, "bundler", &clang_env)?;
        }
        WasmPackages::Deno => {
            println!("Packaging Deno WASM target");
            package_wasm_target(&wasm_crate_dir, &pkg_dir, "deno", &clang_env)?;
        }
        WasmPackages::Node => {
            println!("Packaging Node.js WASM target");
            package_wasm_target(&wasm_crate_dir, &pkg_dir, "nodejs", &clang_env)?;
        }
        WasmPackages::Web => {
            println!("Packaging Web WASM target");
            package_wasm_target(&wasm_crate_dir, &pkg_dir, "web", &clang_env)?;
        }
    }
    Ok(())
}

fn package_wasm_target(
    crate_dir: &PathBuf,
    pkg_dir: &Path,
    target: &str,
    clang_env: &[(String, String)],
) -> Result<()> {
    let out_path = pkg_dir.join(target);

    // Remove existing output directory if it exists
    if out_path.exists() {
        fs::remove_dir_all(&out_path)?;
    }

    let mut c = Command::new("wasm-pack");
    c.current_dir(crate_dir);

    let args = vec![
        "build",
        "--target",
        target,
        "--release",
        "--out-dir",
        out_path.to_str().unwrap(),
    ];

    c.args(args);

    // Set clang environment variables if provided
    for (key, value) in clang_env {
        c.env(key, value);
    }

    let status = c
        .status()
        .with_context(|| format!("failed to build wasm target {}", target))?;

    if !status.success() {
        bail!("wasm-pack build failed for target {}", target);
    }

    // For Node.js target, copy the JavaScript sqlite storage implementation
    if target == "nodejs" {
        copy_nodejs_storage_files(crate_dir, &out_path)?;
    }

    if target == "web" || target == "bundler" {
        copy_web_storage_files(crate_dir, &out_path)?;
    }

    println!("Successfully built WASM target: {}", target);
    Ok(())
}

fn copy_nodejs_storage_files(crate_dir: &Path, out_path: &Path) -> Result<()> {
    let js_storage_src = crate_dir.join("js/node-storage");

    if !js_storage_src.exists() {
        println!(
            "Warning: Node.js storage source directory not found at {:?}",
            js_storage_src
        );
        return Ok(());
    }

    let storage_dest = out_path.join("storage");

    // Create storage directory in output
    std::fs::create_dir_all(&storage_dest)?;

    // Copy the CommonJS storage implementation files (keeping .cjs extensions)
    let files_to_copy = ["index.cjs", "errors.cjs", "migrations.cjs"];

    for file_name in files_to_copy {
        let src_file = js_storage_src.join(file_name);
        let dest_file = storage_dest.join(file_name);

        if src_file.exists() {
            std::fs::copy(&src_file, &dest_file).with_context(|| {
                format!(
                    "Failed to copy {} to {}",
                    src_file.display(),
                    dest_file.display()
                )
            })?;
            println!("Copied CommonJS storage file: {}", file_name);
        } else {
            return Err(anyhow::anyhow!(
                "CommonJS storage file not found: {}",
                src_file.display()
            ));
        }
    }

    // Create a CommonJS package.json for the storage module
    let storage_package_json = serde_json::json!({
        "name": "@breez-sdk/node-storage",
        "version": "1.0.0",
        "description": "Node.js SQLite storage implementation for Breez SDK WASM (CommonJS)",
        "main": "index.js",
        "dependencies": {
            "better-sqlite3": ">=8.0.0"
        }
    });

    let dest_package_json = storage_dest.join("package.json");
    let package_content = serde_json::to_string_pretty(&storage_package_json)
        .with_context(|| "Failed to serialize storage package.json")?;

    std::fs::write(&dest_package_json, package_content)
        .with_context(|| "Failed to write storage package.json".to_string())?;
    println!("Created CommonJS storage package.json");

    // Create a modified entry point that includes the storage
    create_nodejs_entry_point(out_path)?;

    // Update the package.json to include storage files and dependencies
    update_nodejs_package_json(out_path)?;

    println!(
        "Successfully copied Node.js storage files to {}",
        storage_dest.display()
    );
    Ok(())
}

fn create_nodejs_entry_point(out_path: &Path) -> Result<()> {
    let entry_content = r#"// Node.js entry point for Breez SDK with automatic storage support
const wasmModule = require('./breez_sdk_spark_wasm.js');

// Automatically import and set up the storage for Node.js
try {
    const { createDefaultStorage } = require('./storage/index.cjs');
    
    // Make createDefaultStorage available globally for WASM to find
    global.createDefaultStorage = createDefaultStorage;
    
    console.log('Breez SDK: Node.js storage automatically enabled');
} catch (error) {
    console.warn('Breez SDK: Failed to load Node.js storage:', error.message);
    console.warn('Breez SDK: Storage operations may not work properly. Ignore this warning if you are not using the default storage.');
}

// Export all WASM functions
module.exports = wasmModule;
"#;

    let dts_content = r#"export * from "./breez_sdk_spark_wasm.js";"#;

    let entry_file = out_path.join("index.js");
    std::fs::write(&entry_file, entry_content).with_context(|| {
        format!(
            "Failed to create Node.js entry point at {}",
            entry_file.display()
        )
    })?;

    let dts_file = out_path.join("index.d.ts");
    std::fs::write(&dts_file, dts_content).with_context(|| {
        format!(
            "Failed to create Node.js .d.ts entry point at {}",
            dts_file.display()
        )
    })?;

    println!("Created Node.js entry point with automatic storage setup");
    Ok(())
}

fn update_nodejs_package_json(out_path: &Path) -> Result<()> {
    let package_json_path = out_path.join("package.json");

    // Read the current package.json
    let package_json_content = std::fs::read_to_string(&package_json_path).with_context(|| {
        format!(
            "Failed to read package.json at {}",
            package_json_path.display()
        )
    })?;

    // Parse as JSON to modify it
    let mut package_json: serde_json::Value = serde_json::from_str(&package_json_content)
        .with_context(|| "Failed to parse package.json")?;

    // Update the main entry point
    package_json["main"] = serde_json::Value::String("index.js".to_string());

    package_json["types"] = serde_json::Value::String("index.d.ts".to_string());

    // Add storage files to the files array
    if let Some(files) = package_json.get_mut("files") {
        if let Some(files_array) = files.as_array_mut() {
            files_array.push(serde_json::Value::String("storage/".to_string()));
            files_array.push(serde_json::Value::String("index.js".to_string()));
        }
    } else {
        package_json["files"] = serde_json::json!([
            "breez_sdk_spark_wasm_bg.wasm",
            "breez_sdk_spark_wasm.js",
            "breez_sdk_spark_wasm.d.ts",
            "storage/",
            "index.js"
        ]);
    }

    // Write the updated package.json
    let updated_content = serde_json::to_string_pretty(&package_json)
        .with_context(|| "Failed to serialize package.json")?;

    std::fs::write(&package_json_path, updated_content).with_context(|| {
        format!(
            "Failed to write updated package.json at {}",
            package_json_path.display()
        )
    })?;

    println!("Updated Node.js package.json with storage configuration");
    Ok(())
}

fn copy_web_storage_files(crate_dir: &Path, out_path: &Path) -> Result<()> {
    let js_storage_src = crate_dir.join("js/web-storage");

    if !js_storage_src.exists() {
        println!(
            "Warning: Web storage source directory not found at {:?}",
            js_storage_src
        );
        return Ok(());
    }

    let storage_dest = out_path.join("storage");

    // Create storage directory in output
    std::fs::create_dir_all(&storage_dest)?;

    // Copy the ES6 storage implementation files
    let files_to_copy = ["index.js"];

    for file_name in files_to_copy {
        let src_file = js_storage_src.join(file_name);
        let dest_file = storage_dest.join(file_name);

        if src_file.exists() {
            std::fs::copy(&src_file, &dest_file).with_context(|| {
                format!(
                    "Failed to copy {} to {}",
                    src_file.display(),
                    dest_file.display()
                )
            })?;
            println!("Copied ES6 web storage file: {}", file_name);
        } else {
            return Err(anyhow::anyhow!(
                "ES6 web storage file not found: {}",
                src_file.display()
            ));
        }
    }

    // Create an ES6 package.json for the web storage module
    let storage_package_json = serde_json::json!({
        "name": "@breez-sdk/web-storage",
        "version": "1.0.0",
        "description": "Web IndexedDB storage implementation for Breez SDK WASM (ES6 modules)",
        "type": "module",
        "main": "index.js",
        "exports": {
            ".": "./index.js",
            "./storage": "./index.js",
        },
        "dependencies": {}
    });

    let dest_package_json = storage_dest.join("package.json");
    let package_content = serde_json::to_string_pretty(&storage_package_json)
        .with_context(|| "Failed to serialize web storage package.json")?;

    std::fs::write(&dest_package_json, package_content)
        .with_context(|| "Failed to write web storage package.json".to_string())?;
    println!("Created CommonJS web storage package.json");

    // Create a modified entry point that includes the storage
    create_web_entry_point(out_path)?;

    // Update the package.json to include storage files
    update_web_package_json(out_path)?;

    println!(
        "Successfully copied Web storage files to {}",
        storage_dest.display()
    );
    Ok(())
}

fn create_web_entry_point(out_path: &Path) -> Result<()> {
    let entry_content = r#"// Web/Browser entry point for Breez SDK with automatic IndexedDB storage support
import wasmInit, * as wasmModule from './breez_sdk_spark_wasm.js';

// Automatically import and set up the IndexedDB storage for web/browser environments
let storageSetupComplete = false;

const setupWebStorage = async () => {
    if (storageSetupComplete) return;
    
    try {
        // Dynamic import of storage module
        const { createDefaultStorage } = await import('./storage/index.js');
        
        // Make createDefaultStorage available globally for WASM to find
        globalThis.createDefaultStorage = createDefaultStorage;
        
        console.log('Breez SDK: Web IndexedDB storage automatically enabled');
        storageSetupComplete = true;
    } catch (error) {
        console.warn('Breez SDK: Failed to load Web storage:', error.message);
        console.warn('Breez SDK: Storage operations may not work properly. Ignore this warning if you are not using the default storage.');
    }
};

// Initialize WASM and storage
const initBreezSDK = async () => {
    await setupWebStorage();
    return await wasmInit();
};

// Export the initialization function and all WASM functions
export default initBreezSDK;
export * from './breez_sdk_spark_wasm.js';
"#;

    let dts_content = r#"export * from "./breez_sdk_spark_wasm.js";
export default function initBreezSDK(): Promise<void>;
    "#;

    let entry_file = out_path.join("index.js");
    std::fs::write(&entry_file, entry_content).with_context(|| {
        format!(
            "Failed to create Web entry point at {}",
            entry_file.display()
        )
    })?;

    let dts_file = out_path.join("index.d.ts");
    std::fs::write(&dts_file, dts_content).with_context(|| {
        format!(
            "Failed to create Web .d.ts entry point at {}",
            dts_file.display()
        )
    })?;

    println!("Created Web entry point with automatic IndexedDB storage setup");
    Ok(())
}

fn update_web_package_json(out_path: &Path) -> Result<()> {
    let package_json_path = out_path.join("package.json");

    // Read the current package.json
    let package_json_content = std::fs::read_to_string(&package_json_path).with_context(|| {
        format!(
            "Failed to read package.json at {}",
            package_json_path.display()
        )
    })?;

    // Parse as JSON to modify it
    let mut package_json: serde_json::Value = serde_json::from_str(&package_json_content)
        .with_context(|| "Failed to parse package.json")?;

    // Update the main entry point
    package_json["main"] = serde_json::Value::String("index.js".to_string());
    package_json["module"] = serde_json::Value::String("index.js".to_string());

    // Add browser-specific exports
    package_json["exports"] = serde_json::json!({
        ".": {
            "import": "./index.js",
            "default": "./index.js"
        },
        "./storage": {
            "import": "./storage/index.js",
            "default": "./storage/index.js"
        }
    });

    package_json["types"] = serde_json::Value::String("index.d.ts".to_string());

    // Add storage files to the files array
    if let Some(files) = package_json.get_mut("files") {
        if let Some(files_array) = files.as_array_mut() {
            files_array.push(serde_json::Value::String("storage/".to_string()));
            files_array.push(serde_json::Value::String("index.js".to_string()));
        }
    } else {
        package_json["files"] = serde_json::json!([
            "breez_sdk_spark_wasm_bg.wasm",
            "breez_sdk_spark_wasm.js",
            "breez_sdk_spark_wasm.d.ts",
            "storage/",
            "index.js"
        ]);
    }

    // Ensure dependencies section exists (even if empty for web)
    if package_json.get("dependencies").is_none() {
        package_json["dependencies"] = serde_json::json!({});
    }

    // Add browser-specific fields
    package_json["browser"] = serde_json::Value::String("index.js".to_string());

    // Add sideEffects false for better tree shaking
    package_json["sideEffects"] = serde_json::Value::Bool(false);

    // Write the updated package.json
    let updated_content = serde_json::to_string_pretty(&package_json)
        .with_context(|| "Failed to serialize package.json")?;

    std::fs::write(&package_json_path, updated_content).with_context(|| {
        format!(
            "Failed to write updated package.json at {}",
            package_json_path.display()
        )
    })?;

    println!("Updated Web package.json with storage configuration");
    Ok(())
}
