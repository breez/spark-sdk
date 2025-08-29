{
  description = "Kotlin flake";
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-24.05";
    flake-utils.url = "github:numtide/flake-utils";
  };
  outputs =
    { self
    , nixpkgs
    , flake-utils
    ,
    }:
    flake-utils.lib.eachDefaultSystem (system:
    let
      pkgs = nixpkgs.legacyPackages.${system};
    in
    {
      devShells.default = pkgs.mkShell {
        # TODO make gradle use java17 by default.
        buildInputs = with pkgs; [
          kotlin
          ktlint
        ];
      };

      formatter = pkgs.nixpkgs-fmt;
    });
}
