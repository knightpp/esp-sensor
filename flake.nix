{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-23.05";
    nixpkgs-esp-dev.url = "github:mirrexagon/nixpkgs-esp-dev";
  };

  outputs = { self, nixpkgs, nixpkgs-esp-dev }:
    let
      # Systems supported
      allSystems = [
        "x86_64-linux"
        # Not supported
        # "aarch64-linux"
        # "x86_64-darwin"
        # "aarch64-darwin"
      ];

      forAllSystems = f: nixpkgs.lib.genAttrs allSystems (system: f {
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ (import "${nixpkgs-esp-dev}/overlay.nix") ];
        };
      });
    in
    {
      devShells = forAllSystems ({ pkgs }: {
        default = pkgs.mkShell {
          name = "esp32";
          buildInputs = with pkgs; [
            esp-idf-esp32
            rustup
          ];
          shellHook = ''
            . .env
            . ~/export-esp.sh
          '';
        };
      });
    };
}
