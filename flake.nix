{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-23.05";
    esp-dev.url = "github:mirrexagon/nixpkgs-esp-dev";
    esp-dev.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = {
    self,
    nixpkgs,
    esp-dev,
  }: let
    pkgs = import nixpkgs {
      system = "x86_64-linux";
      overlays = [(import "${esp-dev}/overlay.nix")];
    };
  in {
    devShells.x86_64-linux.default = pkgs.mkShell {
      name = "dev shell";

      env = {
        MCU = "esp32c3";
      };

      # embuild does not work with IDF sources without .git directory, so to use local clone just
      # unset IDF_PATH
      shellHook = ''
        unset IDF_PATH
        fish
      '';

      buildInputs = builtins.attrValues {
        inherit
          (pkgs)
          rustup
          pkg-config
          python3
          git
          gcc
          ninja
          cmake
          esp-idf-esp32c3
          ;
      };
    };
  };
}
