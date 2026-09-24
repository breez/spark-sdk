{ pkgs, craneLib }:

let
  packages = import ./packages.nix { inherit pkgs craneLib; };
  mempool = import ./mempool.nix { inherit pkgs; };
in
{
  packages = {
    inherit (packages)
      spark-operator
      spark-frost-signer
      ldk-server
      sspd
      lnurl
      data-sync
      mempool-electrs;
    mempool-backend = mempool.backend;
    mempool-frontend = mempool.frontend;
    local-env = import ./local-env.nix { inherit pkgs packages mempool; };
  };
}
