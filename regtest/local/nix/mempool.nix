# The mempool explorer at the release the environment's docker images carry.
{ pkgs }:

let
  inherit (pkgs) lib;

  version = "3.3.1";

  src = pkgs.fetchFromGitHub {
    owner = "mempool";
    repo = "mempool";
    rev = "v${version}";
    hash = "sha256-Py+ou6xwgentp1PNluDNPjw+gdWM3HxWVPHewuo/5hE=";
  };

  nodejs = pkgs.nodejs_22;

  frontendConfig = pkgs.writeText "mempool-frontend-config.json" (builtins.toJSON {
    ROOT_NETWORK = "regtest";
    REGTEST_ENABLED = true;
    MAINNET_ENABLED = false;
    MINING_DASHBOARD = false;
  });
in
rec {
  # The block template builder the backend loads as a native module.
  rust-gbt = pkgs.stdenv.mkDerivation {
    pname = "mempool-rust-gbt";
    inherit version src;
    sourceRoot = "${src.name}/rust/gbt";
    cargoDeps = pkgs.rustPlatform.fetchCargoVendor {
      inherit src;
      sourceRoot = "${src.name}/rust/gbt";
      hash = "sha256-eox/K3ipjAqNyFt87lZnxaU/okQLF/KIhqXrX86n+qw=";
    };
    nativeBuildInputs = [
      pkgs.rustPlatform.cargoSetupHook
      pkgs.cargo
      pkgs.rustc
      pkgs.napi-rs-cli
    ];
    buildPhase = ''
      runHook preBuild
      napi build --platform --release --strip out
      runHook postBuild
    '';
    installPhase = ''
      mkdir -p $out
      cp out/* package.json $out/
    '';
  };

  backend = pkgs.buildNpmPackage {
    pname = "mempool-backend";
    inherit version src nodejs;
    sourceRoot = "${src.name}/backend";
    npmDepsHash = "sha256-x8QQhwhEbZlE2h9RYJLmuggXqxO8OBI9Ru+ihlmVkpo=";
    # Its preinstall builds rust-gbt, which is built above instead.
    npmFlags = [ "--ignore-scripts" ];
    postPatch = ''
      cp -r ${rust-gbt} rust-gbt
      chmod -R u+w rust-gbt
    '';
    MEMPOOL_COMMIT_HASH = "v${version}";
    nativeBuildInputs = [ pkgs.makeWrapper ];
    installPhase = ''
      runHook preInstall
      mkdir -p $out/lib/mempool-backend $out/bin
      cp -r dist/. package.json $out/lib/mempool-backend/
      cp -r node_modules $out/lib/mempool-backend/
      rm $out/lib/mempool-backend/node_modules/rust-gbt
      cp -r rust-gbt $out/lib/mempool-backend/node_modules/rust-gbt
      makeWrapper ${nodejs}/bin/node $out/bin/mempool-backend \
        --add-flags $out/lib/mempool-backend/index.js
      runHook postInstall
    '';
  };

  frontend = pkgs.buildNpmPackage {
    pname = "mempool-frontend";
    inherit version src nodejs;
    sourceRoot = "${src.name}/frontend";
    npmDepsHash = "sha256-PSvuWah65F4XSm7/2/aZZXd4FETLXC2nCl0Gb8vZRcU=";
    npmFlags = [ "--ignore-scripts" ];
    nativeBuildInputs = [ pkgs.rsync ];
    # sync-assets.js downloads mining pool logos and the like; the explorer runs
    # without them. Each translation adds minutes to the build, so only the
    # source locale is built.
    postPatch = ''
      cp ${frontendConfig} mempool-frontend-config.json
      : > sync-assets.js
      ${pkgs.jq}/bin/jq '.projects.mempool.i18n.locales = {}' angular.json > angular.json.new
      mv angular.json.new angular.json
    '';
    env = {
      CI = "true";
      NG_CLI_ANALYTICS = "false";
    };
    # `npm run build` without its progress spinner, which floods the build log.
    buildPhase = ''
      runHook preBuild
      npm run generate-themes
      npm run generate-config
      npm run ng -- build --configuration production --localize --progress=false
      npm run sync-assets
      runHook postBuild
    '';
    installPhase = ''
      runHook preInstall
      cp -r dist/mempool/browser $out
      runHook postInstall
    '';
  };
}
