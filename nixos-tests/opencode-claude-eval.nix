{
  pkgs,
  fleetixLib,
}: let
  inherit (pkgs) lib;
  # Exercise both nixpkgs download contracts, including the legacy recipe's
  # hardcoded raw filename. Checksums are pinned independently of the selector.
  platform = "${pkgs.stdenv.hostPlatform.node.platform}-${pkgs.stdenv.hostPlatform.node.arch}";
  fixture = compressed:
    lib.makeOverridable ({
      manifest ? {
        version = "2.0.0";
        platforms.${platform} = {
          binary =
            if compressed
            then "claude.zst"
            else "claude";
          checksum = lib.fakeHash;
        };
      },
    }: {
      inherit (manifest) version;
      src = pkgs.fetchurl {
        url = "https://downloads.claude.ai/claude-code-releases/${manifest.version}/${platform}/${
          if compressed
          then manifest.platforms.${platform}.binary
          else "claude"
        }";
        sha256 = manifest.platforms.${platform}.checksum;
      };
    }) {};
  selectClient = client: (pkgs.callPackage ../nix/opencode-with-claude {claude-code = client;}).claudePackage;
  rawClient = selectClient (fixture false);
  compressedClient = selectClient (fixture true);
  currentClient = pkgs.claude-code.overrideAttrs {version = "999.0.0";};
  expectedHashes = {
    linux-x64 = {
      raw = "5cd90aabd83f8a15136c35aa37bb1d92b348993573316643dc3fe4e04afbf88f";
      compressed = "5021d631dacbd516603a779b3cf2463085470417a838b6a0835f77fa44d23f0a";
    };
    linux-arm64 = {
      raw = "3dd0f96d7ada463152d20300186f6cfc6ab94b57e218f49e3ac86db42ac695a6";
      compressed = "6e31b86de3952594441b4ef91c1f5079c1385b0169a9df351dd2762bb2336d23";
    };
  };
  base = {
    options = {
      assertions = lib.mkOption {
        type = lib.types.listOf lib.types.attrs;
        default = [];
      };
      programs.opencode = {
        enable = lib.mkEnableOption "OpenCode";
        settings = lib.mkOption {
          inherit ((pkgs.formats.json {})) type;
          default = {};
        };
      };
      home.homeDirectory = lib.mkOption {
        type = lib.types.str;
        default = "/home/example";
      };
      xdg.configHome = lib.mkOption {
        type = lib.types.str;
        default = "/home/example/.config";
      };
      systemd.user.services = lib.mkOption {
        type = lib.types.attrs;
        default = {};
      };
    };
  };
  topology.services.endpoints = {
    claude = {
      targetHost = "example";
      port = 3460;
      bind = "loopback";
      transport = "http";
    };
    jev = {
      targetHost = "example";
      port = 8792;
      bind = "loopback";
      transport = "http";
    };
  };
  evaluate = extra:
    (lib.evalModules {
      specialArgs = {inherit pkgs;};
      modules = [base (import ../modules/home/opencode-claude.nix {inherit fleetixLib;}) extra];
    }).config;
  enabled = evaluate {
    programs.opencode.enable = true;
    canix-toolbelt.opencodeJev = {
      enable = true;
      package = pkgs.emptyDirectory;
      credentialFile = "%t/agenix/typesafe";
    };
    canix-toolbelt.opencodeClaude = {
      enable = true;
      package = pkgs.emptyDirectory;
      inherit topology;
      hostName = "example";
      meridianEndpoint = "claude";
      jevEndpoint = "jev";
    };
  };
  direct = evaluate {
    programs.opencode.enable = true;
    canix-toolbelt.opencodeClaude = {
      enable = true;
      package = pkgs.emptyDirectory;
      inherit topology;
      hostName = "example";
      meridianEndpoint = "claude";
    };
  };
  gatewayDisabled = evaluate {
    programs.opencode.enable = true;
    canix-toolbelt.opencodeClaude = {
      enable = true;
      package = pkgs.emptyDirectory;
      inherit topology;
      hostName = "example";
      meridianEndpoint = "claude";
      jevEndpoint = "jev";
    };
  };
  invalid = evaluate {
    programs.opencode.enable = true;
    canix-toolbelt.opencodeJev = {
      enable = true;
      package = pkgs.emptyDirectory;
      credentialFile = "/runtime/key";
    };
    canix-toolbelt.opencodeClaude = {
      enable = true;
      package = pkgs.emptyDirectory;
      topology.services.endpoints = topology.services.endpoints // {claude = topology.services.endpoints.claude // {bind = "lan";};};
      hostName = "example";
      meridianEndpoint = "claude";
      jevEndpoint = "jev";
    };
  };
  missing = evaluate {
    canix-toolbelt.opencodeClaude = {
      enable = true;
      package = pkgs.emptyDirectory;
      inherit topology;
      hostName = "other";
      meridianEndpoint = "claude";
      jevEndpoint = "jev";
    };
  };
in
  assert rawClient.version == "2.1.284" && compressedClient.version == "2.1.284";
  assert rawClient.src.url == "https://downloads.claude.ai/claude-code-releases/2.1.284/${platform}/claude";
  assert compressedClient.src.url == "https://downloads.claude.ai/claude-code-releases/2.1.284/${platform}/claude.zst";
  assert rawClient.src.outputHash == expectedHashes.${platform}.raw;
  assert compressedClient.src.outputHash == expectedHashes.${platform}.compressed;
  assert (selectClient currentClient).drvPath == currentClient.drvPath;
  assert (evaluate {}).programs.opencode.settings == {};
  assert lib.all (a: a.assertion) enabled.assertions;
  assert lib.all (a: a.assertion) direct.assertions;
  assert !(lib.all (a: a.assertion) gatewayDisabled.assertions);
  assert direct.canix-toolbelt.opencodeJev.gateways == {};
  assert direct.canix-toolbelt.opencodeJev.units == [];
  assert builtins.attrNames direct.systemd.user.services == ["meridian-opencode"];
  assert builtins.length direct.programs.opencode.settings.plugins == 1;
  assert direct.programs.opencode.settings.providers.anthropic.settings.baseURL == "http://127.0.0.1:3460/v1";
  assert !(lib.all (a: a.assertion) invalid.assertions);
  assert !(builtins.tryEval (builtins.deepSeq missing.assertions true)).success;
  assert enabled.programs.opencode.settings.providers.anthropic.settings.baseURL == "http://127.0.0.1:3460/v1";
  assert (builtins.head enabled.programs.opencode.settings.plugins).options.externalBaseURL == "http://127.0.0.1:3460";
  assert (lib.last enabled.programs.opencode.settings.plugins).options.routes."http://127.0.0.1:3460/v1/messages" == "http://127.0.0.1:8792/v1/messages";
  assert enabled.systemd.user.services.jev-gateway-anthropic.Unit.Requires == ["meridian-opencode.service"];
  assert builtins.elem "HOME=/home/example" enabled.systemd.user.services.meridian-opencode.Service.Environment;
  assert builtins.elem "CLAUDE_CONFIG_DIR" enabled.systemd.user.services.meridian-opencode.Service.UnsetEnvironment;
  assert builtins.elem "ANTHROPIC_BASE_URL" enabled.systemd.user.services.meridian-opencode.Service.UnsetEnvironment;
  assert enabled.systemd.user.services.jev-gateway-anthropic.Service.LoadCredential == "typesafe:%t/agenix/typesafe";
    pkgs.writeText "opencode-claude-eval" "ok"
