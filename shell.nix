let
  pkgs = import <nixpkgs> {};
in
  pkgs.stdenv.mkDerivation {
    name = "dev-env";
    buildInputs = [
      pkgs.rustup
      pkgs.pkg-config
    ];
  }
