# The environment's programs, built from the same pins as its docker images.
{ pkgs, craneLib }:

let
  inherit (pkgs) lib;

  dockerDir = ../../../crates/spark-itest/docker;

  # The commit a dockerfile's `ARG VERSION=` pins, so both setups build the same
  # operator and Lightning node.
  pinnedVersion = dockerfile:
    builtins.head (builtins.match ".*\nARG VERSION=([0-9a-f]+)\n.*" (builtins.readFile dockerfile));

  sparkSrc = builtins.fetchGit {
    url = "https://github.com/breez/spark.git";
    rev = pinnedVersion (dockerDir + "/spark-so.dockerfile");
    shallow = true;
  };

  # The data-sync commit .env pins, so both setups run the same service.
  dataSyncSrc = builtins.fetchGit {
    url = "https://github.com/breez/data-sync.git";
    rev = builtins.head (builtins.match ".*\nDATA_SYNC_VERSION=([0-9a-f]+)\n.*" (builtins.readFile ../.env));
    shallow = true;
  };

  ldkServerSrc = builtins.fetchGit {
    url = "https://github.com/breez/ldk-server.git";
    rev = pinnedVersion (dockerDir + "/ldk-server.dockerfile");
    shallow = true;
  };

  buildGo127Module = pkgs.buildGoModule.override { go = pkgs.go_1_27; };

  repoRoot = ../../..;
in
rec {
  spark-operator = buildGo127Module {
    pname = "spark-operator";
    version = builtins.substring 0 9 sparkSrc.rev;
    src = sparkSrc;
    modRoot = "spark";
    subPackages = [ "bin/operator" ];
    # Moves with the operator pin in spark-so.dockerfile.
    vendorHash = "sha256-LyEzEfL29dXoHsKa7agQBJWokDyiKkDfe5yFeQrPK08=";
    nativeBuildInputs = [ pkgs.pkg-config ];
    buildInputs = [ pkgs.zeromq ];
    doCheck = false;
  };

  spark-frost-signer = pkgs.rustPlatform.buildRustPackage {
    pname = "spark-frost-signer";
    version = builtins.substring 0 9 sparkSrc.rev;
    src = sparkSrc;
    cargoRoot = "signer";
    buildAndTestSubdir = "signer/spark-frost-signer";
    # Moves with the operator pin in spark-so.dockerfile.
    cargoHash = "sha256-E9THpU4MBMrSmp2cNAMmWgL4FOLR303fH3Sp4Q3DGSI=";
    nativeBuildInputs = [ pkgs.protobuf ];
    doCheck = false;
  };

  spark-migrations = "${sparkSrc}/spark/so/ent/migrate/migrations";

  ldk-server = pkgs.rustPlatform.buildRustPackage {
    pname = "ldk-server";
    version = builtins.substring 0 9 ldkServerSrc.rev;
    src = ldkServerSrc;
    cargoBuildFlags = [ "-p" "ldk-server" "-p" "ldk-server-cli" ];
    # Moves with the pin in ldk-server.dockerfile.
    cargoHash = "sha256-7x4t7lx9Vd/OKhfGvGW4HHSKj9JgGib3vvtgolcoClA=";
    doCheck = false;
  };

  # The workspace's own daemons, sharing one build of their dependencies.
  workspaceBin =
    let
      src = lib.fileset.toSource {
        root = repoRoot;
        fileset = lib.fileset.unions [
          (repoRoot + "/Cargo.toml")
          (repoRoot + "/Cargo.lock")
          (repoRoot + "/crates")
        ];
      };
      common = {
        version = "0.1.0";
        inherit src;
        # Hashed, since a commit off a repository's default branch cannot be
        # fetched without one. Moves with the git dependencies in Cargo.lock.
        cargoVendorDir = craneLib.vendorCargoDeps {
          inherit src;
          outputHashes = {
            "git+https://github.com/breez/boltz-client?rev=aea35af1628d1fb259ebf85266f96215cfdabcb5#aea35af1628d1fb259ebf85266f96215cfdabcb5" = "sha256-CYrCMsbJAX1vJE6OnNSsvc6aVmv2MfShZ9st2Z15TXw=";
            "git+https://github.com/breez/uniffi-rs?branch=v0.29.5-breez#f1133c3c7aedb4135c58ac51caeebc2d6cdf0b44" = "sha256-M0t6936nZhRCmUDoqfUX3kDKkf4kqLmudWDvyuU2Mjw=";
            "git+https://github.com/lightsparkdev/frost?rev=9aaf1b6b9fa3c2c3c2c7c70da83061deda1a9180#9aaf1b6b9fa3c2c3c2c7c70da83061deda1a9180" = "sha256-U8KCu1wVofNQBeN4zuIgmxN5lSKj/6YXu7J4o5oIvbQ=";
          };
        };
        strictDeps = true;
        nativeBuildInputs = [ pkgs.protobuf ];
        doCheck = false;
        # The workspace's release profile optimizes the SDK's size, at a cost in
        # build time a local daemon has no use for.
        CARGO_PROFILE_RELEASE_LTO = "false";
        CARGO_PROFILE_RELEASE_OPT_LEVEL = "2";
        CARGO_PROFILE_RELEASE_CODEGEN_UNITS = "16";
      };
      crateArgs = crates: "--locked " + lib.concatMapStringsSep " " (crate: "-p ${crate}") crates;
      cargoArtifacts = craneLib.buildDepsOnly (common // {
        pname = "spark-sdk-deps";
        cargoExtraArgs = crateArgs [ "sspd" "ssp-cli" "lnurl" ];
        # The dummy workspace keeps no tests, and cargo refuses a member without
        # a target: macro_test has only tests.
        extraDummyScript = ''
          mkdir -p $out/crates/macro_test/tests
          touch $out/crates/macro_test/tests/dummy.rs
        '';
      });
    in
    { pname, crates }: craneLib.buildPackage (common // {
      inherit pname cargoArtifacts;
      cargoExtraArgs = crateArgs crates;
    });

  sspd = workspaceBin {
    pname = "sspd";
    crates = [ "sspd" "ssp-cli" ];
  };

  lnurl = workspaceBin {
    pname = "lnurl";
    crates = [ "lnurl" ];
  };

  data-sync = buildGo127Module {
    pname = "data-sync";
    version = builtins.substring 0 9 dataSyncSrc.rev;
    src = dataSyncSrc;
    # Moves with DATA_SYNC_VERSION in .env.
    vendorHash = "sha256-h6YTNM8nDsme1r2y+6oSG0JKOMWjdPFbB1hEkaxHHI4=";
    doCheck = false;
  };

  mempool-electrs = pkgs.rustPlatform.buildRustPackage rec {
    pname = "mempool-electrs";
    version = "3.3.0";
    src = pkgs.fetchFromGitHub {
      owner = "mempool";
      repo = "electrs";
      rev = "v${version}";
      hash = "sha256-oxeD/z+jCe1dG9tmgYy5AUJKCuX3QNErR5gIARhhoZY=";
    };
    cargoHash = "sha256-evmJeF3zrfhvFv4kYArFzqR+AiDO+U95AKmyvuyKA30=";
    nativeBuildInputs = [ pkgs.rustPlatform.bindgenHook ];
    doCheck = false;
  };
}
