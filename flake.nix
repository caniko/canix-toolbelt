{
  description = "canix-toolbelt — reusable, host-agnostic Nix modules and flake-parts modules extracted from caniko/canix";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";
    treefmt-nix = {
      url = "github:numtide/treefmt-nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    home-manager = {
      url = "github:nix-community/home-manager";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    # dns-manager owns reusable DNS schema/rendering/backend code. Toolbelt
    # keeps only host/service-registry integration and the canix DNS wrapper.
    dns-manager = {
      url = "git+https://codeberg.org/caniko/dns-manager.git?rev=c4abb508275cb1f105363953047bf22f015c0292";
      inputs = {
        nixpkgs.follows = "nixpkgs";
        treefmt-nix.follows = "treefmt-nix";
      };
    };
    fleetix = {
      url = "git+https://codeberg.org/caniko/fleetix.git";
      inputs.nixpkgs.follows = "nixpkgs";
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
    # Patched crush fork with api_key_file support for NixOS/agenix deployments.
    crush-fork = {
      url = "github:caniko/crush/feat/api-key-file";
      flake = false;
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
            # Compatibility alias. Fleetix owns these projections; keep the
            # old export name during the migration without carrying a second
            # implementation in canix-toolbelt.
            fleetix = inputs.fleetix.lib;
            # ACP/agent provider preset package sets for programs.goose; pass
            # the host `pkgs`. Lets hosts wire e.g.
            #   acp.providers.claude.packages =
            #     canix-toolbelt.lib.goose.acpPackages pkgs).claude;
            goose.acpPackages = import ./modules/home/ai/goose/presets.nix;
            gooseCheckFixtures = import ./modules/home/ai/goose/checks/lib.nix;
            chromiumGpu = import ./lib/chromiumGpu.nix {
              lib = nixpkgs.lib;
              wrapper-manager = inputs.wrapper-manager;
            };
          };
        nixosModules = import ./modules/nixos;
        homeModules = import ./modules/home {inherit (inputs) wrapper-manager goose;};
        flakeModules = {
          agenix-rekey-auto = ./flake-modules/agenix-rekey-auto.nix;
          caddy-helpers = ./flake-modules/caddy-helpers.nix;
          dev-stack = ./flake-modules/dev-stack.nix;
          formatters = ./flake-modules/formatters.nix;
          git-hooks = ./flake-modules/git-hooks.nix;
          host-selection = ./flake-modules/host-selection.nix;
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
            chromium-gpu-eval = import ./nixos-tests/chromium-gpu-eval.nix {inherit inputs pkgs;};
            dns-apex-cname-assertion = import ./nixos-tests/dns-apex-cname-assertion.nix {inherit inputs pkgs;};
            dns-caddy-redirect-routes = import ./nixos-tests/dns-caddy-redirect-routes.nix {inherit inputs pkgs;};
            dns-lib-helpers-eval = import ./nixos-tests/dns-lib-helpers-eval.nix {inherit pkgs;};
            dns-octodns-apply-force = import ./nixos-tests/dns-octodns-apply-force.nix {inherit inputs pkgs;};
            activation-contracts-eval = import ./nixos-tests/activation-contracts-eval.nix {inherit pkgs;};
            activation-manifest-eval = import ./nixos-tests/activation-manifest-eval.nix {inherit pkgs;};
            agent-safety-eval = import ./nixos-tests/agent-safety-eval.nix {inherit pkgs;};
            agent-safety-home-eval = import ./nixos-tests/agent-safety-home-eval.nix {inherit pkgs;};
            dev-agent-isolation-eval = import ./nixos-tests/dev-agent-isolation-eval.nix {inherit pkgs;};
            pg-backup-eval = import ./nixos-tests/pg-backup-eval.nix {inherit pkgs;};
            host-selection-eval = import ./nixos-tests/host-selection-eval.nix {inherit pkgs;};
            caddy-service-registry-oidc = import ./nixos-tests/caddy-service-registry-oidc.nix {inherit pkgs;};
            kanidm-preset-eval = import ./nixos-tests/kanidm-preset-eval.nix {inherit pkgs;};
            rauthy-preset-eval = import ./nixos-tests/rauthy-preset-eval.nix {inherit pkgs;};
            site-helpers-eval = import ./nixos-tests/site-helpers-eval.nix {inherit pkgs;};
          }
          // import ./nixos-tests/nexus-profiles.nix {inherit pkgs;};

        packages.website = website;
        packages.site = website;
        packages.crush = let
          # Transitive dep charm.land/fantasy requires go >= 1.26.4.
          go_1_26_4 = pkgs.go.overrideAttrs (_old: {
            version = "1.26.4";
            src = pkgs.fetchurl {
              url = "https://go.dev/dl/go1.26.4.linux-amd64.tar.gz";
              hash = "sha256-EVPT1Q4Kx2S0R63+BcK88I6InUKgLg/gJZvUf2czrX8=";
            };
          });
        in
          pkgs.buildGoModule.override {go = go_1_26_4;} {
            pname = "crush";
            version = "0.77.0-api-key-file";
            src = inputs.crush-fork;
            vendorHash = "sha256-a+4k+fjqdWsAUv0ilagd46pYwFaSd1+mJ25Vr47Lsys=";
            ldflags = ["-s" "-w"];
            doCheck = false;
          };
        apps.deploy-pages = inputs.plinth.lib.${system}.mkDeployPagesApp {
          domain = "canix-toolbelt.tartanoglu.com";
        };
        formatter = pkgs.alejandra;
      };
    };
}
