{
  description = "canix-toolbelt — reusable, host-agnostic Nix modules and flake-parts modules extracted from caniko/canix";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";
    treefmt-nix.url = "github:numtide/treefmt-nix";
    home-manager = {
      url = "github:nix-community/home-manager";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    # dns-manager owns reusable DNS schema/rendering/backend code. Toolbelt
    # keeps only host/service-registry integration and the canix DNS wrapper.
    dns-manager = {
      url = "git+ssh://git@codeberg.org/caniko/dns-manager.git";
      inputs = {
        nixpkgs.follows = "nixpkgs";
        treefmt-nix.follows = "treefmt-nix";
      };
    };
    git-hooks = {
      url = "github:cachix/git-hooks.nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    # Home-manager module helpers (canix-toolbelt.homeModules.chromium-gpu)
    # build their wrappers with wrapper-manager. Pinned to match consumers;
    # wrapper-manager has no nixpkgs input of its own (it takes `pkgs` at the
    # call site), so there is nothing to `follows`.
    wrapper-manager.url = "github:viperML/wrapper-manager/51ad0422b925d830bf4af36979fed51209f79c0a";
    # Upstream goose flake (Home Manager module + Goose Desktop package).
    # Tracks the fork integration branch carrying caniko's not-yet-merged nix
    # PRs (aaif-goose/goose#9517 + #9522); repoint to aaif-goose/goose once
    # they land.
    goose = {
      url = "github:caniko/goose/nix/flake-integration";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    plinth = {
      url = "git+https://codeberg.org/caniko/plinth.git?ref=refs/heads/trunk";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    secret-manager = {
      url = "git+https://codeberg.org/caniko/secret-manager.git";
      inputs = {
        nixpkgs.follows = "nixpkgs";
        treefmt-nix.follows = "treefmt-nix";
        git-hooks.follows = "git-hooks";
      };
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
        ./modules/home/ai/goose/checks/core.nix
      ];

      flake = {
        lib =
          (import ./lib {inherit (nixpkgs) lib;})
          // {
            # ACP/agent provider preset package sets for programs.goose; pass
            # the host `pkgs`. Lets hosts wire e.g.
            #   acp.providers.claude.packages =
            #     canix-toolbelt.lib.goose.acpPackages pkgs).claude;
            goose.acpPackages = import ./modules/home/ai/goose/presets.nix;
            gooseCheckFixtures = import ./modules/home/ai/goose/checks/lib.nix;
          };
        nixosModules = import ./modules/nixos;
        homeModules = import ./modules/home {inherit (inputs) wrapper-manager goose;};
        flakeModules = {
          agenix-rekey-auto = ./flake-modules/agenix-rekey-auto.nix;
          caddy-helpers = ./flake-modules/caddy-helpers.nix;
          dev-stack = ./flake-modules/dev-stack.nix;
          formatters = ./flake-modules/formatters.nix;
          git-hooks = ./flake-modules/git-hooks.nix;
          ops-shell = ./flake-modules/ops-shell.nix;
          pages-deploy = ./flake-modules/pages-deploy.nix;
          shebang-audit = ./flake-modules/shebang-audit.nix;
          structure-check = ./flake-modules/structure-check.nix;
          topology = ./flake-modules/topology.nix;
        };
      };

      perSystem = {
        pkgs,
        system,
        ...
      }: let
        website = inputs.plinth.lib.${system}.mkProjectSite {
          pname = "canix-toolbelt-website";
          domain = "canix-toolbelt.tartanoglu.com";
          configPath = ./website/plinth-project.toml;
        };
      in {
        checks =
          {
            forgejo-runner-tls = import ./nixos-tests/forgejo-runner-tls.nix {inherit pkgs;};
            dns-apex-cname-assertion = import ./nixos-tests/dns-apex-cname-assertion.nix {inherit inputs pkgs;};
            kanidm-preset-eval = import ./nixos-tests/kanidm-preset-eval.nix {inherit pkgs;};
            rauthy-preset-eval = import ./nixos-tests/rauthy-preset-eval.nix {inherit pkgs;};
            site-helpers-eval = import ./nixos-tests/site-helpers-eval.nix {inherit pkgs;};
          }
          // import ./nixos-tests/nexus-profiles.nix {inherit pkgs;};

        packages.website = website;
        packages.site = website;
        apps.deploy-pages = inputs.plinth.lib.${system}.mkDeployPagesApp {
          domain = "canix-toolbelt.tartanoglu.com";
        };
        formatter = pkgs.alejandra;
      };
    };
}
