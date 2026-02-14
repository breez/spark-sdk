mod docs;
mod package;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use cargo_metadata::{Metadata, MetadataCommand, Package};
use clap::{Parser, Subcommand};
use xshell::{Shell, cmd};

use crate::docs::{DocSnippetsPackage, check_doc_snippets_cmd};
use crate::package::{TargetPackage, package_cmd};

const OUT_OF_WORKSPACE_PACKAGES: &[&str] = &["crates/breez-sdk/lnurl/Cargo.toml"];
#[derive(Parser, Debug)]
#[command(name = "xtask")]
#[command(about = "Workspace tasks")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Run tests
    Test {
        /// Package name to test; defaults to all workspace members
        #[arg(short = 'p', long = "package")]
        package: Option<String>,

        /// Run only doctests
        #[arg(long)]
        doc: bool,

        /// Extra args passed after `--` to cargo test
        #[arg(last = true)]
        rest: Vec<String>,
    },

    /// Run wasm tests (node/browser) for crates that support wasm
    WasmTest {
        /// Package name to test; defaults to all wasm-capable packages
        #[arg(short = 'p', long = "package")]
        package: Option<String>,

        /// Browser to use for headless tests (firefox|chrome)
        #[arg(long, default_value = "firefox")]
        browser: String,

        /// Run node-based wasm tests instead of browser
        #[arg(long)]
        node: bool,

        /// Extra args passed after `--` to wasm test tool
        #[arg(last = true)]
        rest: Vec<String>,
    },

    /// Run clippy across the workspace
    Clippy {
        /// Apply fixes
        #[arg(long)]
        fix: bool,
        /// Additional args to pass to clippy after `--`
        #[arg(last = true)]
        rest: Vec<String>,
    },

    /// Run clippy for wasm target (wasm32-unknown-unknown)
    WasmClippy {
        /// Apply fixes
        #[arg(long)]
        fix: bool,
        /// Additional args to pass to clippy after `--`
        #[arg(last = true)]
        rest: Vec<String>,
    },

    /// Check formatting
    Fmt {
        /// Check only, do not write changes
        #[arg(long)]
        check: bool,
    },

    /// Build workspace
    Build {
        /// Release build
        #[arg(long)]
        release: bool,
        /// Build for a specific target triple (e.g., wasm32-unknown-unknown)
        #[arg(long)]
        target: Option<String>,
        /// Build only a specific package (otherwise the whole workspace or wasm-only set)
        #[arg(short = 'p', long = "package")]
        package: Option<String>,
    },

    /// Prepares packages
    Package { package: Option<TargetPackage> },

    /// Check doc snippets
    CheckDocSnippets {
        #[arg(short = 'p', long = "package")]
        package: Option<DocSnippetsPackage>,
        #[arg(long, default_value_t = false, action = clap::ArgAction::SetTrue)]
        skip_build: bool,
    },

    /// Run integration tests (containers etc.)
    Itest {},

    /// Check Flutter package (generate bindings and build)
    FlutterCheck {},
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Test { package, doc, rest } => test_cmd(package, doc, rest),
        Commands::WasmTest {
            package,
            browser,
            node,
            rest,
        } => wasm_test_cmd(package, browser, node, rest),
        Commands::Clippy { fix, rest } => clippy_cmd(fix, rest),
        Commands::WasmClippy { fix, rest } => wasm_clippy_cmd(fix, rest),
        Commands::Fmt { check } => fmt_cmd(check),
        Commands::Build {
            release,
            target,
            package,
        } => build_cmd(release, target, package),
        Commands::Package { package } => package_cmd(package),
        Commands::CheckDocSnippets {
            package,
            skip_build,
        } => check_doc_snippets_cmd(package, skip_build),
        Commands::Itest {} => itest_cmd(),
        Commands::FlutterCheck {} => flutter_check_cmd(),
    }
}

fn workspace_metadata() -> Result<Metadata> {
    let meta = MetadataCommand::new().no_deps().exec()?;
    Ok(meta)
}

/// Returns workspace arguments that exclude WASM-only packages from non-WASM operations
fn workspace_exclude_wasm() -> Vec<String> {
    vec!["--exclude".to_string(), "breez-sdk-spark-wasm".to_string()]
}

fn test_cmd(package: Option<String>, doc: bool, rest: Vec<String>) -> Result<()> {
    let mut c = Command::new("cargo");
    c.arg("test");
    c.arg("--no-fail-fast");
    if package.is_none() {
        c.arg("--workspace");
        c.args(["--exclude", "spark-itest"]);
        c.args(["--exclude", "breez-sdk-itest"]);
        c.args(workspace_exclude_wasm());
    }
    if let Some(pkg) = package {
        c.args(["-p", &pkg]);
    }
    if doc {
        c.arg("--doc");
    }
    if !rest.is_empty() {
        c.arg("--").args(&rest);
    }
    let status = c.status().with_context(|| "failed to run cargo test")?;
    if !status.success() {
        bail!("tests failed");
    }
    Ok(())
}

fn packages_with_wasm_tests(meta: &Metadata) -> Vec<Package> {
    meta.packages
        .iter()
        .filter(|p| {
            // Simple manifest content check to detect wasm-bindgen-test in any section
            let manifest = fs::read_to_string(&p.manifest_path).unwrap_or_default();
            manifest.contains("wasm-bindgen-test")
        })
        .cloned()
        .collect()
}

fn find_package<'a>(meta: &'a Metadata, name: &str) -> Result<&'a Package> {
    meta.packages
        .iter()
        .find(|p| p.name == name)
        .ok_or_else(|| anyhow::anyhow!("package '{}' not found", name))
}

fn wasm_test_cmd(
    package: Option<String>,
    browser: String,
    node: bool,
    rest: Vec<String>,
) -> Result<()> {
    let meta = workspace_metadata()?;
    let sh = Shell::new()?;

    // Ensure wasm target exists
    let _ = cmd!(sh, "rustup target add wasm32-unknown-unknown").run();

    // Ensure wasm-pack exists (preferred runner for wasm tests across crates)
    let _ = cmd!(sh, "cargo install wasm-pack --locked").run();

    let packages: Vec<Package> = if let Some(p) = package {
        vec![find_package(&meta, &p)?.clone()]
    } else {
        packages_with_wasm_tests(&meta)
    };

    if packages.is_empty() {
        bail!("No packages with wasm tests found");
    }

    // Prefer cargo test with wasm-bindgen-test runner. Allow browser or node mode
    let mut envs = vec![("RUSTFLAGS".to_string(), String::new())];
    if node {
        // Node is default for wasm-bindgen-test when browser env not set
    } else {
        // Configure headless browser
        let env_browser = match browser.as_str() {
            "firefox" => "firefox",
            "chrome" | "chromium" => "chrome",
            _ => "firefox",
        };
        envs.push((
            "WASM_BINDGEN_TEST_BROWSER".to_string(),
            format!("headless-{env_browser}"),
        ));
    }

    for pkg in packages {
        // If this is the breez-sdk-spark-wasm package, let's install nodejs dependencies and rebuild native modules
        if pkg.name == "breez-sdk-spark-wasm" && node {
            let storage_path = Path::new("crates/breez-sdk/wasm/js/node-storage");

            if !storage_path.exists() {
                bail!(
                    "Native storage directory not found: {}",
                    storage_path.display()
                );
            }

            println!(
                "Installing npm dependencies in {}...",
                storage_path.display()
            );
            let current_dir = std::env::current_dir()?;
            sh.change_dir(storage_path);
            cmd!(sh, "npm install")
                .run()
                .with_context(|| "Failed to install npm dependencies. Ensure npm is installed.")?;

            println!("Rebuilding native modules for current Node.js version...");
            cmd!(sh, "npm rebuild")
                .run()
                .with_context(|| "Failed to rebuild native modules.")?;
            sh.change_dir(&current_dir);
        }

        let package_dir = pkg
            .manifest_path
            .parent()
            .map(|p| p.to_path_buf())
            .expect("manifest has parent");

        let mut c = Command::new("wasm-pack");
        c.current_dir(&package_dir);
        c.arg("test");

        if node {
            c.arg("--node");
        } else {
            c.arg("--headless");
            match browser.as_str() {
                "firefox" => c.arg("--firefox"),
                "chrome" | "chromium" => c.arg("--chrome"),
                _ => c.arg("--firefox"),
            };
            // Enable browser-specific tests if gated behind a feature
            c.args(["--", "--features", "browser-tests"]);
        }
        // On macOS, auto-detect Homebrew LLVM and set CC/AR for wasm cross-compiles
        if cfg!(target_os = "macos")
            && let Some((clang_path, llvm_ar_path)) = detect_brew_llvm_paths(&sh)
        {
            c.env("CC_wasm32_unknown_unknown", &clang_path);
            c.env("AR_wasm32_unknown_unknown", &llvm_ar_path);
        }
        for (k, v) in &envs {
            c.env(k, v);
        }
        if !rest.is_empty() {
            c.args(["--"]).args(&rest);
        }
        let status = c
            .status()
            .with_context(|| "failed to start wasm tests via wasm-pack")?;
        if !status.success() {
            bail!("wasm tests failed");
        }
    }

    Ok(())
}

fn detect_brew_llvm_paths(sh: &Shell) -> Option<(String, String)> {
    let prefix = cmd!(sh, "brew --prefix llvm").read().ok()?;
    let prefix = prefix.trim();
    let clang_path = PathBuf::from(prefix).join("bin").join("clang");
    let llvm_ar_path = PathBuf::from(prefix).join("bin").join("llvm-ar");
    let clang = clang_path.to_str()?.to_string();
    let ar = llvm_ar_path.to_str()?.to_string();
    Some((clang, ar))
}

/// Heuristic to detect crates that are wasm-capable.
/// We consider a crate wasm-capable if its manifest contains either:
/// - A target-specific section for wasm (target_family = "wasm")
/// - Or common wasm-only dependencies present in any section
fn packages_wasm_capable(meta: &Metadata) -> Vec<Package> {
    let wasm_markers = [
        "target_family = \"wasm\"",
        "tonic-web-wasm-client",
        "tokio_with_wasm",
        "wasm-bindgen",
        "wasm-bindgen-futures",
        "wasm-bindgen-test",
    ];
    meta.packages
        .iter()
        .filter(|p| {
            let manifest = fs::read_to_string(&p.manifest_path).unwrap_or_default();
            wasm_markers.iter().any(|m| manifest.contains(m))
        })
        .cloned()
        .collect()
}

fn clippy_cmd(fix: bool, rest: Vec<String>) -> Result<()> {
    let exclude_args = workspace_exclude_wasm();

    // Helper function to run clippy with specific target type
    let run_clippy = |target_type: &str, args: &[String]| -> Result<()> {
        let mut c = Command::new("cargo");
        c.arg("clippy");
        c.arg("--workspace");
        c.arg(target_type);
        if fix {
            c.arg("--fix");
        }
        c.args(&exclude_args);
        c.arg("--");
        c.arg("-D").arg("warnings");
        c.args(args);
        let status = c
            .status()
            .with_context(|| format!("failed to run cargo clippy {target_type}"))?;
        if !status.success() {
            bail!("clippy {target_type} failed");
        }
        Ok(())
    };

    let run_single_crate_clippy = |location: &str, target_type: &str, args: &[String]| {
        let mut c = Command::new("cargo");
        c.arg("clippy");
        c.arg("--manifest-path").arg(location);
        c.arg(target_type);
        if fix {
            c.arg("--fix");
        }
        c.arg("--");
        c.arg("-D").arg("warnings");
        c.args(args);
        let status = c
            .status()
            .with_context(|| format!("failed to run cargo clippy {target_type}"))?;
        if !status.success() {
            bail!("clippy {target_type} failed");
        }
        Ok(())
    };

    // Run clippy for all targets
    run_clippy("--all-targets", &rest)?;
    // Run clippy for tests
    run_clippy("--tests", &rest)?;

    for package in OUT_OF_WORKSPACE_PACKAGES {
        run_single_crate_clippy(package, "--all-targets", &rest)?;
        run_single_crate_clippy(package, "--tests", &rest)?;
    }
    Ok(())
}

fn fmt_cmd(check: bool) -> Result<()> {
    let fmt_cmd = |location: Option<&str>| {
        let mut c = Command::new("cargo");
        c.arg("fmt").arg("--all");
        if check {
            c.arg("--check");
        }

        if let Some(location) = location {
            c.arg("--manifest-path").arg(location);
        }

        let status = c
            .status()
            .with_context(|| "failed to run cargo fmt".to_string())?;

        if !status.success() {
            bail!("fmt failed");
        }

        Ok(())
    };

    fmt_cmd(None)?;
    for package in OUT_OF_WORKSPACE_PACKAGES {
        fmt_cmd(Some(package))?;
    }
    Ok(())
}

fn build_cmd(release: bool, target: Option<String>, package: Option<String>) -> Result<()> {
    let sh = Shell::new()?;
    match target {
        None => match package {
            None => {
                let exclude_args = workspace_exclude_wasm();
                if release {
                    let mut c = Command::new("cargo");
                    c.arg("build");
                    c.arg("--workspace");
                    c.arg("--release");
                    c.args(&exclude_args);
                    let status = c
                        .status()
                        .with_context(|| "failed to run cargo build --workspace --release")?;
                    if !status.success() {
                        bail!("build --workspace --release failed");
                    }
                } else {
                    let mut c = Command::new("cargo");
                    c.arg("build");
                    c.arg("--workspace");
                    c.args(&exclude_args);
                    let status = c
                        .status()
                        .with_context(|| "failed to run cargo build --workspace")?;
                    if !status.success() {
                        bail!("build --workspace failed");
                    }
                }
            }
            Some(p) => {
                if release {
                    cmd!(sh, "cargo build -p {p} --release").run()?;
                } else {
                    cmd!(sh, "cargo build -p {p}").run()?;
                }
            }
        },
        Some(t) => {
            // Ensure target toolchain is installed
            let _ = cmd!(sh, "rustup target add {t}").run();

            // If building for wasm and no package specified, only build detected wasm-capable packages
            let packages_to_build: Vec<String> =
                if package.is_none() && t == "wasm32-unknown-unknown" {
                    let meta = workspace_metadata()?;
                    let wasm_pkgs = packages_wasm_capable(&meta);
                    if wasm_pkgs.is_empty() {
                        Vec::new()
                    } else {
                        wasm_pkgs.into_iter().map(|p| p.name).collect()
                    }
                } else {
                    match &package {
                        None => Vec::new(), // build whole workspace
                        Some(p) => vec![p.clone()],
                    }
                };

            if packages_to_build.is_empty() {
                // Build whole workspace for the target
                let mut c = Command::new("cargo");
                c.arg("build");
                c.arg("--workspace");
                c.args(workspace_exclude_wasm());
                c.arg("--target").arg(&t);
                if release {
                    c.arg("--release");
                }
                if cfg!(target_os = "macos")
                    && t == "wasm32-unknown-unknown"
                    && let Some((clang_path, llvm_ar_path)) = detect_brew_llvm_paths(&sh)
                {
                    c.env("CC_wasm32_unknown_unknown", &clang_path);
                    c.env("AR_wasm32_unknown_unknown", &llvm_ar_path);
                }
                let status = c
                    .status()
                    .with_context(|| "failed to run cargo build for target")?;
                if !status.success() {
                    bail!("build failed for target {t}");
                }
            } else {
                for p in packages_to_build {
                    let mut c = Command::new("cargo");
                    c.arg("build");
                    c.args(["-p", &p]);
                    c.arg("--target").arg(&t);
                    if release {
                        c.arg("--release");
                    }
                    if cfg!(target_os = "macos")
                        && t == "wasm32-unknown-unknown"
                        && let Some((clang_path, llvm_ar_path)) = detect_brew_llvm_paths(&sh)
                    {
                        c.env("CC_wasm32_unknown_unknown", &clang_path);
                        c.env("AR_wasm32_unknown_unknown", &llvm_ar_path);
                    }
                    let status = c
                        .status()
                        .with_context(|| format!("failed to build package {p} for target"))?;
                    if !status.success() {
                        bail!("build failed for package {p} target {t}");
                    }
                }
            }
        }
    }
    Ok(())
}

fn run_wasm_clippy_for_package(
    sh: &Shell,
    pkg_name: &str,
    target: &str,
    fix: bool,
    rest: &[String],
) -> Result<()> {
    let mut c = Command::new("cargo");
    c.arg("clippy");
    c.args(["-p", pkg_name]);
    c.arg("--all-targets");
    c.arg("--target").arg(target);

    if fix {
        c.arg("--fix");
    }
    c.arg("--");
    c.arg("-D").arg("warnings");
    if !rest.is_empty() {
        for r in rest {
            c.arg(r);
        }
    }
    if cfg!(target_os = "macos")
        && let Some((clang_path, llvm_ar_path)) = detect_brew_llvm_paths(sh)
    {
        c.env("CC_wasm32_unknown_unknown", &clang_path);
        c.env("AR_wasm32_unknown_unknown", &llvm_ar_path);
    }
    let status = c
        .status()
        .with_context(|| format!("failed to run cargo clippy for wasm target on {pkg_name}"))?;
    if !status.success() {
        bail!("wasm clippy failed for {pkg_name}");
    }
    Ok(())
}

fn wasm_clippy_cmd(fix: bool, rest: Vec<String>) -> Result<()> {
    let sh = Shell::new()?;
    // Ensure wasm target exists
    let target = "wasm32-unknown-unknown".to_string();
    let _ = cmd!(sh, "rustup target add {target}").run();

    // Detect wasm-capable workspace packages
    let meta = workspace_metadata()?;
    let wasm_packages = packages_wasm_capable(&meta);
    if wasm_packages.is_empty() {
        bail!("No wasm-capable packages detected in workspace");
    }
    for pkg in wasm_packages.iter() {
        run_wasm_clippy_for_package(&sh, &pkg.name, &target, fix, &rest)?;
    }
    Ok(())
}

fn itest_cmd() -> Result<()> {
    let sh = Shell::new()?;

    // Verify Docker is available
    cmd!(sh, "docker --version").run().with_context(
        || "docker is required for integration tests; please install and start Docker Desktop",
    )?;

    // Pull base images used by tests
    if let Err(e) = cmd!(sh, "docker image pull lncm/bitcoind:v28.0").run() {
        println!(
            "Failed to pull lncm/bitcoind:v28.0, continuing anyway, it might exist locally already: {e}"
        );
    }
    if let Err(e) = cmd!(sh, "docker image pull postgres:11-alpine").run() {
        println!(
            "Failed to pull postgres:11-alpine, continuing anyway, it might exist locally already: {e}"
        );
    }

    // Build local images from crates/spark-itest/docker
    let workspace_root = std::env::current_dir()?;
    let docker_dir = workspace_root.join("crates/spark-itest/docker");
    let docker_dir_str = docker_dir
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("invalid workspace path"))?;

    let migrations_df = docker_dir.join("migrations.dockerfile");
    let spark_so_df = docker_dir.join("spark-so.dockerfile");
    let migrations_df_str = migrations_df
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("invalid migrations.dockerfile path"))?;
    let spark_so_df_str = spark_so_df
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("invalid spark-so.dockerfile path"))?;

    cmd!(
        sh,
        "docker build -t spark-migrations -f {migrations_df_str} {docker_dir_str}"
    )
    .run()?;
    cmd!(
        sh,
        "docker build -t spark-so -f {spark_so_df_str} {docker_dir_str}"
    )
    .run()?;

    // Run the integration tests
    cmd!(sh, "cargo test -p spark-itest --no-fail-fast").run()?;
    Ok(())
}

fn flutter_check_cmd() -> Result<()> {
    let workspace_root = std::env::current_dir()?;
    let flutter_dir = workspace_root.join("packages/flutter");

    println!("Checking Flutter package...");
    let status = Command::new("just")
        .arg("build")
        .current_dir(&flutter_dir)
        .status()
        .with_context(|| "failed to run `just build`. Make sure just is installed: cargo install just (or brew install just)")?;
    if !status.success() {
        bail!("Flutter check failed: just build failed");
    }

    println!("Flutter check completed successfully");
    Ok(())
}
