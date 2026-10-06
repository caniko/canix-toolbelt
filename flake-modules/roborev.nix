{
  nixpkgs,
  harborGo,
  harborJs,
  harborMeta,
  homeManager,
}: {
  flake.homeManagerModules.roborev = ../modules/home/roborev;
  perSystem = {system, ...}: let
    pkgs = nixpkgs.legacyPackages.${system};
    package = import ../nix/roborev.nix {
      inherit pkgs;
      inherit harborGo harborJs;
    };
    goFragment = harborGo.lib.mkGoDevShellFragment {
      inherit pkgs;
      inherit (package) toolchain;
      cgo = true;
    };
    shell = harborMeta.lib.devShell.mkShell {
      inherit pkgs;
      fragments = [
        goFragment
        {
          inherit (package.bunToolchain) packages env;
        }
      ];
      packages = [pkgs.git pkgs.gnumake pkgs.sqlite];
    };
    moduleResults = import ../tests/roborev/eval.nix {
      inherit pkgs;
      inherit homeManager;
    };
  in
    pkgs.lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux {
      packages =
        {
          roborev = package;
          roborev-web = package.frontend;
          roborev-bun-deps = package.deps;
        }
        // pkgs.lib.optionalAttrs (system == "x86_64-linux") {
          roborev-aarch64 = import ../nix/roborev.nix {
            inherit harborGo harborJs;
            pkgs = pkgs.pkgsCross.aarch64-multiplatform;
            # Cross package sets can rebuild native tools with a different
            # derivation identity. Reuse this output's exact native tool set.
            buildPkgs = pkgs.buildPackages;
          };
        };
      devShells.roborev = shell;
      checks =
        {
          roborev-module = assert pkgs.lib.assertMsg
          (builtins.all (value: value) (builtins.attrValues moduleResults))
          "roborev module regression failed; evaluate tests/roborev/eval.nix for individual results";
            pkgs.writeText "roborev-module-results.json" (builtins.toJSON moduleResults);
          roborev-shell = harborMeta.lib.devShellTests.mkCheck {
            inherit pkgs shell;
            name = "roborev-shell";
            commands = ["go" "gopls" "golangci-lint" "bun" "git" "make" "sqlite3"];
            env = {
              GOTOOLCHAIN = "local";
              CGO_ENABLED = "1";
              BUN_VERSION = "1.3.14";
            };
          };
        }
        // pkgs.lib.optionalAttrs (system == "x86_64-linux") {
          roborev-unit = import ../tests/roborev/unit-vm.nix {
            inherit pkgs;
            inherit homeManager;
          };
        };
    };
}
