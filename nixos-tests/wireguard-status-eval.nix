{pkgs}: let
  inherit (pkgs) lib;
  inherit (import ./lib/eval-checks.nix {inherit pkgs;}) mkEvalCheck;

  evaluate = settings:
    (import "${pkgs.path}/nixos/lib/eval-config.nix" {
      inherit pkgs;
      system = null;
      modules = [
        ../modules/nixos/networking/wireguard-status.nix
        {
          boot.isContainer = true;
          system.stateVersion = "25.05";
          canix-toolbelt.networking.wireguard-status = settings;
        }
      ];
    }).config;

  disabled = evaluate {};
  enabled = evaluate {
    enable = true;
    port = 19586;
    interfaces = ["wg-home"];
  };
  custom = evaluate {
    enable = true;
    interfaces = ["wg-home"];
    listenAddress = "127.0.0.2";
    withRemoteIp = false;
    statusFile = "/etc/custom/wireguard-status.json";
  };
  invalidPaths = [
    "/var/lib/wg.json"
    "/etc/"
    "/etc/../tmp/pwn.json"
    "/etc/foo/../bar.json"
    "/etc/foo/"
    "/etc//foo.json"
    "/etc/."
    "/etc/.."
  ];
  invalidConfigs =
    map (path: {
      inherit path;
      cfg = evaluate {
        enable = true;
        statusFile = path;
      };
    })
    invalidPaths;

  enabledJson = builtins.fromJSON enabled.environment.etc."canix-toolbelt/wireguard-status.json".text;
  customJson = builtins.fromJSON custom.environment.etc."custom/wireguard-status.json".text;
in
  mkEvalCheck {
    name = "wireguard-status-eval";
    resultMessage = "wireguard-status NixOS module evaluation passed";
    assertions = [
      {
        name = "enabled-exporter";
        assertion =
          enabled.services.prometheus.exporters.wireguard.enable
          && enabled.services.prometheus.exporters.wireguard.listenAddress == "127.0.0.1"
          && enabled.services.prometheus.exporters.wireguard.port == 19586
          && enabled.services.prometheus.exporters.wireguard.withRemoteIp == true
          && enabled.services.prometheus.exporters.wireguard.interfaces == ["wg-home"];
        message = "enabled wireguard-status must configure services.prometheus.exporters.wireguard";
      }
      {
        name = "enabled-default-status-file";
        assertion =
          enabledJson
          == {
            schemaVersion = 1;
            metricsUrl = "http://127.0.0.1:19586/metrics";
            interfaces = ["wg-home"];
          }
          && lib.all (a: a.assertion) enabled.assertions;
        message = "default statusFile must render the expected JSON descriptor and pass all assertions";
      }
      {
        name = "custom-status-file-and-options";
        assertion =
          (custom.environment.etc ? "custom/wireguard-status.json")
          && !(custom.environment.etc ? "canix-toolbelt/wireguard-status.json")
          && custom.services.prometheus.exporters.wireguard.listenAddress == "127.0.0.2"
          && custom.services.prometheus.exporters.wireguard.withRemoteIp == false
          && customJson
          == {
            schemaVersion = 1;
            metricsUrl = "http://127.0.0.2:9586/metrics";
            interfaces = ["wg-home"];
          }
          && lib.all (a: a.assertion) custom.assertions;
        message = "custom statusFile, listenAddress, and withRemoteIp must be applied and pass assertions";
      }
      {
        name = "invalid-status-files-fail-assertion-cleanly";
        assertion = lib.all (entry:
          lib.any (a: !a.assertion && lib.hasInfix "wireguard-status.statusFile" a.message) entry.cfg.assertions
          && !(builtins.hasAttr (lib.removePrefix "/etc/" entry.path) entry.cfg.environment.etc))
        invalidConfigs;
        message = "invalid statusFile paths (outside /etc/, empty, traversal, or trailing/double slashes) must fail assertion without emitting an environment.etc entry";
      }
      {
        name = "disabled-leaves-exporter-and-etc-unset";
        assertion =
          !disabled.services.prometheus.exporters.wireguard.enable
          && !(disabled.environment.etc ? "canix-toolbelt/wireguard-status.json");
        message = "disabled wireguard-status must not enable the exporter or create the etc file";
      }
    ];
  }
