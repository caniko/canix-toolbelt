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
    disko = {
      url = "github:nix-community/disko";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    # dns-manager owns reusable DNS schema/rendering/backend code. Toolbelt
    # keeps only host/service-registry integration and the canix DNS wrapper.
    dns-manager = {
      url = "git+https://github.com/caniko/dns-manager.git";
      inputs = {
        nixpkgs.follows = "nixpkgs";
        treefmt-nix.follows = "treefmt-nix";
      };
    };
    fleetix = {
      url = "git+https://github.com/caniko/fleetix.git?ref=trunk&rev=2230d9ee804a66d94424a91919182e4fcca13ab2";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    # Reuse the locked compiler tooling; Rust library dependencies remain Cargo-owned.
    harbor-rs.follows = "fleetix/harbor-rs";
    # Transitional compatibility only: database-specific modules now live in
    # db-harbor and this input can be removed after consumers migrate.
    harbor-db = {
      url = "git+https://github.com/caniko/harbor-db.git";
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
    plinth = {
      url = "git+https://github.com/caniko/plinth.git?ref=refs/heads/trunk";
    };
    secret-manager = {
      url = "git+https://github.com/caniko/secret-manager.git";
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
        ./flake-modules/rust.nix
      ];

      flake = {
        lib =
          (import ./lib {
            inherit (nixpkgs) lib;
            fleetixLib = inputs.fleetix.lib;
          })
          // {
            # Compatibility alias. Fleetix owns these projections; keep the
            # old export name during the migration without carrying a second
            # implementation in canix-toolbelt.
            fleetix = inputs.fleetix.lib;
            chromiumGpu = import ./lib/chromiumGpu.nix {
              inherit (nixpkgs) lib;
              inherit (inputs) wrapper-manager;
            };
            firefoxGpu = import ./lib/firefoxGpu.nix {
              inherit (nixpkgs) lib;
              inherit (inputs) wrapper-manager;
            };
          };
        nixosModules =
          (import ./modules/nixos {fleetixGpu = inputs.fleetix.lib.gpu;})
          // {
            cloud-host = {
              imports = [
                inputs.disko.nixosModules.disko
                ./modules/nixos/hosts/cloud-host.nix
              ];
            };
            # Compatibility alias. Database-specific backup mechanics are
            # owned by db-harbor; keep the old option path during migration.
            pg-backup = {
              imports = [
                inputs.harbor-db.nixosModules.pg-backup
                (nixpkgs.lib.mkAliasOptionModule
                  ["canix-toolbelt" "services" "pgBackup"]
                  ["services" "db-harbor" "pgBackup"])
              ];
            };
          };
        homeModules = import ./modules/home {
          inherit (inputs) wrapper-manager;
          fleetixGpu = inputs.fleetix.lib.gpu;
        };
        flakeModules = {
          agenix-rekey-auto = ./flake-modules/agenix-rekey-auto.nix;
          caddy-helpers = ./flake-modules/caddy-helpers.nix;
          dev-stack = ./flake-modules/dev-stack.nix;
          formatters = ./flake-modules/formatters.nix;
          git-hooks = ./flake-modules/git-hooks.nix;
          host-selection = ./flake-modules/host-selection.nix;
          ops-shell = ./flake-modules/ops-shell.nix;
          shebang-audit = ./flake-modules/shebang-audit.nix;
          structure-check = ./flake-modules/structure-check.nix;
          topology = ./flake-modules/topology.nix;
        };
      };

      perSystem = {
        config,
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
            cloud-host-eval = import ./nixos-tests/cloud-host-eval.nix {inherit inputs pkgs;};
            direnv-eval = import ./nixos-tests/direnv-eval.nix {inherit pkgs;};
            public-edge-eval = import ./nixos-tests/public-edge-eval.nix {inherit inputs pkgs;};
            garage-buckets-registry-eval = import ./nixos-tests/garage-buckets-registry-eval.nix {inherit pkgs;};
            gatus-instances-eval = assert import ./nixos-tests/gatus-instances-eval.nix {
              inherit pkgs;
              fleetixLib = inputs.fleetix.lib;
            };
              pkgs.writeText "gatus-instances-eval" "ok";
            gatus-publisher-eval = builtins.deepSeq (import ./nixos-tests/gatus-publisher-eval.nix {inherit pkgs;}) (pkgs.writeText "gatus-publisher-eval" "ok");
            attic-projects-registry-eval = import ./nixos-tests/attic-projects-registry-eval.nix {inherit pkgs;};
            chromium-gpu-eval = import ./nixos-tests/chromium-gpu-eval.nix {inherit inputs pkgs;};
            gpu-media-eval = import ./nixos-tests/gpu-media-eval.nix {inherit inputs pkgs;};
            gpu-render-eval = import ./nixos-tests/gpu-render-eval.nix {inherit inputs pkgs;};
            gpu-routes-eval = import ./nixos-tests/gpu-routes-eval.nix {inherit inputs pkgs;};
            direct-link-eval = import ./nixos-tests/direct-link-eval.nix {inherit pkgs;};
            gpu-backends-eval = import ./nixos-tests/gpu-backends-eval.nix {inherit inputs pkgs;};
            dns-apex-cname-assertion = import ./nixos-tests/dns-apex-cname-assertion.nix {inherit inputs pkgs;};
            dns-caddy-redirect-routes = import ./nixos-tests/dns-caddy-redirect-routes.nix {inherit inputs pkgs;};
            dns-lib-helpers-eval = import ./nixos-tests/dns-lib-helpers-eval.nix {inherit pkgs;};
            dns-octodns-apply-force = import ./nixos-tests/dns-octodns-apply-force.nix {inherit inputs pkgs;};
            dns-pages-zone-eval = import ./nixos-tests/dns-pages-zone-eval.nix {inherit inputs pkgs;};
            dns-publication-eval = import ./nixos-tests/dns-publication-eval.nix {inherit inputs pkgs;};
            activation-contracts-eval = import ./nixos-tests/activation-contracts-eval.nix {inherit pkgs;};
            activation-manifest-eval = import ./nixos-tests/activation-manifest-eval.nix {inherit pkgs;};
            resumable-operator-eval = import ./nixos-tests/resumable-operator-eval.nix {inherit pkgs;};
            agent-safety-eval = import ./nixos-tests/agent-safety-eval.nix {inherit pkgs;};
            agent-safety-home-eval = import ./nixos-tests/agent-safety-home-eval.nix {inherit pkgs;};
            browser-connection-eval = import ./nixos-tests/browser-connection-eval.nix {inherit pkgs;};
            browser-connection-smoke = import ./nixos-tests/browser-connection-smoke.nix {inherit pkgs;};
            browser-connection-runtime =
              pkgs.runCommand "browser-connection-runtime" {
                nativeBuildInputs = [pkgs.python3];
              } ''
                cp -r ${./runtime} runtime
                mkdir lib
                cp ${./lib/browserConnection.nix} lib/browserConnection.nix
                python -m unittest discover -s runtime -p test_browser_connection.py -v
                touch "$out"
              '';
            keyboard-layout-shortcut-eval = import ./nixos-tests/keyboard-layout-shortcut-eval.nix {inherit pkgs;};
            dev-agent-isolation-eval = import ./nixos-tests/dev-agent-isolation-eval.nix {inherit pkgs;};
            dev-agent-isolation-scope = import ./nixos-tests/dev-agent-isolation-scope.nix {inherit pkgs;};
            dev-oom-guard-pause-eval = import ./nixos-tests/dev-oom-guard-pause-eval.nix {inherit pkgs;};
            dev-oom-guard-protection = import ./nixos-tests/dev-oom-guard-protection.nix {inherit pkgs;};
            project-tree-eval = import ./nixos-tests/project-tree-eval.nix {inherit pkgs;};
            host-selection-eval = import ./nixos-tests/host-selection-eval.nix {inherit pkgs;};
            wg-home-endpoint-selection = import ./nixos-tests/wg-home-endpoint-selection.nix {inherit pkgs;};
            wg-home-shared-eval = import ./nixos-tests/wg-home-shared-eval.nix {inherit pkgs;};
            ssh-aliases-eval = import ./nixos-tests/ssh-aliases-eval.nix {inherit pkgs;};
            service-topology-v2-eval = import ./nixos-tests/service-topology-v2-eval.nix {inherit inputs pkgs;};
            kanidm-preset-eval = import ./nixos-tests/kanidm-preset-eval.nix {inherit pkgs;};
            rauthy-preset-eval = import ./nixos-tests/rauthy-preset-eval.nix {inherit pkgs;};
            site-helpers-eval = import ./nixos-tests/site-helpers-eval.nix {inherit pkgs;};
            vpn-netns-eval = import ./nixos-tests/vpn-netns-eval.nix {inherit pkgs;};
          }
          // pkgs.lib.optionalAttrs (system == "x86_64-linux") {
            public-edge = import ./nixos-tests/public-edge.nix {inherit pkgs;};
            cloud-host-install-bios = import ./nixos-tests/cloud-host-install.nix {
              inherit inputs pkgs;
              mode = "bios";
            };
            cloud-host-install-uefi = import ./nixos-tests/cloud-host-install.nix {
              inherit inputs pkgs;
              mode = "uefi";
            };
          }
          // import ./nixos-tests/nexus-profiles.nix {inherit pkgs;};

        packages.website = website;
        packages.opencode-browser-adapter = ((import ./lib/browserConnection.nix {inherit (pkgs) lib;}).mkAdapter {inherit pkgs;}).package;
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
        devShells.default = pkgs.mkShell {
          packages = [config.treefmt.build.wrapper pkgs.cargo pkgs.rustc pkgs.clippy pkgs.rustfmt pkgs.cargo-audit pkgs.cmake pkgs.pkg-config pkgs.perl];
          # This shell uses stable Rust; do not inherit Canix's nightly-only flags.
          RUSTFLAGS = "";
          CARGO_ENCODED_RUSTFLAGS = "";
          shellHook = config.pre-commit.installationScript;
        };
        devShells.docs = config.devShells.default;
        formatter = config.treefmt.build.wrapper;
      };
    };
}
