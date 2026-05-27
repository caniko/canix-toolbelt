{
  description = "canix-toolbelt — reusable, host-agnostic Nix modules and flake-parts modules extracted from caniko/canix";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";
    treefmt-nix.url = "github:numtide/treefmt-nix";
    # TODO: swap to upstream github:Janik-Haag/NixOS-DNS after Phase 07 PR #2 merges.
    nixos-dns = {
      url = "git+https://codeberg.org/caniko/NixOS-DNS?ref=feat/cloudflare-backend";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    git-hooks = {
      url = "github:cachix/git-hooks.nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = inputs @ {
    flake-parts,
    nixpkgs,
    ...
  }:
    flake-parts.lib.mkFlake {inherit inputs;} {
      systems = ["x86_64-linux" "aarch64-linux"];

      imports = [
        ./flake-modules/dev-stack.nix
      ];

      flake = {
        lib = import ./lib {inherit (nixpkgs) lib;};
        nixosModules = import ./modules/nixos;
        flakeModules = {
          agenix-rekey-auto = ./flake-modules/agenix-rekey-auto.nix;
          caddy-helpers = ./flake-modules/caddy-helpers.nix;
          dev-stack = ./flake-modules/dev-stack.nix;
          formatters = ./flake-modules/formatters.nix;
          git-hooks = ./flake-modules/git-hooks.nix;
          ops-shell = ./flake-modules/ops-shell.nix;
          shebang-audit = ./flake-modules/shebang-audit.nix;
          structure-check = ./flake-modules/structure-check.nix;
          topology = ./flake-modules/topology.nix;
        };
      };

      perSystem = {pkgs, ...}: {
        checks = {
          forgejo-runner-tls = import ./nixos-tests/forgejo-runner-tls.nix {inherit pkgs;};
        };

        formatter = pkgs.alejandra;
      };
    };
}
