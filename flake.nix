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
    harbor-meta.follows = "harbor-rs/harbor-meta";
    harbor-go = {
      url = "github:caniko/harbor-go/9818c51b9c8864fafa439c038360a383aa54da5b";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.harbor-meta.follows = "harbor-meta";
    };
    harbor-js = {
      url = "github:caniko/harbor-js/25935487646992db132557f526a8b00c460ccfdc";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.harbor-meta.follows = "harbor-meta";
    };
    # Transitional compatibility only: database-specific modules now live in
    # harbor-db; keep the compatibility facade on its qualified storage release.
    harbor-db = {
      url = "github:caniko/harbor-db/99aca6956890ca347a06fe73ca93bbe22fa72368";
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
    # Source-only compatibility qualification; never select this as a V2 runtime.
    opencode-environment-legacy = {
      url = "github:caniko/opencode/b79c099b61ed0e67b5020844367cb6c1ba61c1eb";
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
        (import ./flake-modules/roborev.nix {
          inherit (inputs) nixpkgs;
          harborGo = inputs.harbor-go;
          harborJs = inputs.harbor-js;
          harborMeta = inputs.harbor-meta;
          homeManager = inputs.home-manager;
        })
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
            # owned by harbor-db; keep the old option path during migration.
            pg-backup = {
              imports = [
                inputs.harbor-db.nixosModules.pg-backup
                (nixpkgs.lib.mkAliasOptionModule
                  ["canix-toolbelt" "services" "pgBackup"]
                  ["services" "harbor-db" "pgBackup"])
              ];
            };
            postgres-lifecycle = inputs.harbor-db.nixosModules.postgres-lifecycle;
          };
        homeModules = import ./modules/home {
          inherit (inputs) wrapper-manager;
          fleetixGpu = inputs.fleetix.lib.gpu;
          fleetixLib = inputs.fleetix.lib;
        };
        flakeModules = {
          agenix-rekey-auto = ./flake-modules/agenix-rekey-auto.nix;
          caddy-helpers = ./flake-modules/caddy-helpers.nix;
          dev-stack = ./flake-modules/dev-stack.nix;
          formatters = ./flake-modules/formatters.nix;
          git-hooks = ./flake-modules/git-hooks.nix;
          host-selection = ./flake-modules/host-selection.nix;
          ops-shell = ./flake-modules/ops-shell.nix;
          roborev = {inputs, ...}: {
            imports = [
              (import ./flake-modules/roborev.nix {
                inherit (inputs) nixpkgs;
                harborGo = inputs.harbor-go;
                harborJs = inputs.harbor-js;
                harborMeta = inputs.harbor-meta;
                homeManager = inputs.home-manager;
              })
            ];
          };
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
        # The Claude subscription output explicitly depends on this proprietary
        # executable. Keep its permission pure and limited to that package.
        _module.args.pkgs = import nixpkgs {
          inherit system;
          config.allowUnfreePredicate = package: nixpkgs.lib.getName package == "claude-code";
        };

        checks =
          {
            build-train-eval = import ./nixos-tests/build-train-eval.nix {inherit pkgs;};
            direct-network-eval = import ./nixos-tests/direct-network-eval.nix {inherit pkgs inputs;};
            direct-network-transitions = import ./nixos-tests/direct-network-transitions.nix {inherit pkgs inputs;};
            build-train-policy = assert import ./nixos-tests/build-train-policy.nix != {}; pkgs.writeText "build-train-policy-parity" "ok";
            cloud-host-eval = import ./nixos-tests/cloud-host-eval.nix {inherit inputs pkgs;};
            direnv-eval = import ./nixos-tests/direnv-eval.nix {inherit pkgs;};
            public-edge-eval = import ./nixos-tests/public-edge-eval.nix {inherit inputs pkgs;};
            opencode-environment-eval = import ./nixos-tests/opencode-environment-eval.nix {inherit pkgs;};
            opencode-claude-eval = import ./nixos-tests/opencode-claude-eval.nix {
              inherit pkgs;
              fleetixLib = inputs.fleetix.lib;
            };
            opencode-jev-runtime = pkgs.runCommand "opencode-jev-runtime" {nativeBuildInputs = [pkgs.nodejs];} ''
              node --test ${./runtime/opencode-jev}/server.test.mjs
              touch "$out"
            '';
            opencode-environment-runtime = pkgs.runCommand "opencode-environment-runtime" {nativeBuildInputs = [pkgs.nodejs];} ''
              node --test ${./runtime/opencode-environment}/server.test.mjs
              touch "$out"
            '';
            opencode-environment-legacy = pkgs.runCommand "opencode-environment-legacy" {nativeBuildInputs = [pkgs.nodejs pkgs.git];} ''
              node ${./runtime/opencode-environment}/check-legacy.mjs ${inputs.opencode-environment-legacy}
              touch "$out"
            '';
            garage-buckets-registry-eval = import ./nixos-tests/garage-buckets-registry-eval.nix {inherit pkgs;};
            gatus-instances-eval = assert import ./nixos-tests/gatus-instances-eval.nix {
              inherit pkgs;
              fleetixLib = inputs.fleetix.lib;
            };
              pkgs.writeText "gatus-instances-eval" "ok";
            gatus-publisher-eval = builtins.deepSeq (import ./nixos-tests/gatus-publisher-eval.nix {inherit pkgs;}) (pkgs.writeText "gatus-publisher-eval" "ok");
            attic-projects-registry-eval = import ./nixos-tests/attic-projects-registry-eval.nix {inherit pkgs;};
            harbor-db-compat-eval = import ./nixos-tests/harbor-db-compat-eval.nix {
              inherit pkgs;
              modules = inputs.self.nixosModules;
            };
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
            direnv-runtime = import ./nixos-tests/direnv-runtime.nix {inherit pkgs;};
            resumable-operator-runtime = import ./nixos-tests/resumable-operator-runtime.nix {inherit pkgs;};
            harbor-db-compat-runtime = import ./nixos-tests/harbor-db-compat-runtime.nix {
              inherit pkgs;
              modules = inputs.self.nixosModules;
            };
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
        packages.opencode-with-claude = pkgs.callPackage ./nix/opencode-with-claude {};
        packages.canix-toolbelt = import ./nix/package.nix {inherit pkgs;};
        packages.canix-toolbelt-direct-network = import ./nix/package.nix {
          inherit pkgs;
          directNetwork = true;
        };
        packages.canix-toolbelt-build-train = import ./nix/package.nix {
          inherit pkgs;
          buildTrain = true;
        };
        packages.canix-toolbelt-roborev-worker = import ./nix/roborev-worker.nix {inherit pkgs;};
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
        devShells.roborev-worker = pkgs.mkShell {
          inputsFrom = [config.devShells.default];
          packages = [pkgs.git pkgs.bubblewrap pkgs.python3];
          RUSTFLAGS = "";
          CARGO_ENCODED_RUSTFLAGS = "";
        };
        formatter = config.treefmt.build.wrapper;
      };
    };
}
