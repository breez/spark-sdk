{
  description = "Breez SDK for Spark";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    crane.url = "github:ipetkov/crane";
  };

  outputs = { nixpkgs, crane, ... }:
    let
      systems = [ "aarch64-darwin" "x86_64-darwin" "aarch64-linux" "x86_64-linux" ];
      forEachSystem = f: nixpkgs.lib.genAttrs systems (system:
        let pkgs = nixpkgs.legacyPackages.${system};
        in f (import ./regtest/local/nix { inherit pkgs; craneLib = crane.mkLib pkgs; }));
    in
    {
      packages = forEachSystem (localEnv: localEnv.packages);
      apps = forEachSystem (localEnv: {
        local-env = {
          type = "app";
          program = "${localEnv.packages.local-env}/bin/spark-local";
        };
      });
    };
}
