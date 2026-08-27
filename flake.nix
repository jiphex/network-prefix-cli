{
  description = "Inspect, split and carve up IPv4 and IPv6 prefixes";

  # Only nixpkgs. flake-utils would save a few lines of boilerplate below, but
  # this repository keeps its dependencies to what it actually needs, and the
  # boilerplate is five lines.
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      forAllSystems = f:
        nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});

      # Read straight from the manifest rather than repeating it. The release
      # workflow treats Cargo.toml as the one place a version lives, and a
      # second copy here would be a second thing to forget.
      manifest = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package;
    in
    {
      packages = forAllSystems (pkgs: rec {
        prefixtool = pkgs.rustPlatform.buildRustPackage {
          pname = manifest.name;
          inherit (manifest) version;

          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;

          meta = {
            inherit (manifest) description;
            homepage = manifest.repository;
            license = pkgs.lib.licenses.mit;
            mainProgram = manifest.name;
            platforms = pkgs.lib.platforms.unix;
          };
        };
        default = prefixtool;
      });

      apps = forAllSystems (pkgs: rec {
        prefixtool = {
          type = "app";
          program = "${self.packages.${pkgs.system}.prefixtool}/bin/prefixtool";
        };
        default = prefixtool;
      });

      # `nix flake check` builds the package, and buildRustPackage runs the
      # test suite as part of that, so this is the same suite CI runs.
      checks = forAllSystems (pkgs: {
        prefixtool = self.packages.${pkgs.system}.prefixtool;
      });

      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          packages = with pkgs; [
            cargo
            rustc
            clippy
            rustfmt
            rust-analyzer
          ];
          # Points rust-analyzer and friends at the standard library.
          RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
        };
      });
    };
}
