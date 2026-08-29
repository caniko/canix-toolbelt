{pkgs, ...}: let
  inherit (pkgs) lib;
  inherit (import ./lib/eval-checks.nix {inherit pkgs;}) mkEvalCheck;

  toolbeltLib = import ../lib {inherit lib;};

  goodModule = toolbeltLib.nexus.mkProfilesForHost {
    hostName = "client";
    deviceType = "laptop";
    hostToggles = {
      foo = {
        description = "Foo profile";
        enable = true;
        deviceTypes = ["laptop"];
        specialisations.foo-off.enable = false;
      };

      bar = {
        description = "Bar profile";
        enable = false;
        specialisations = {
          bar-off.enable = false;
          bar-on.enable = true;
        };
      };
    };
  };

  badEval = pkgs.writeText "nexus-device-types-failure.nix" ''
    let
      toolbeltLib = import ${../lib} { lib = import ${pkgs.path}/lib; };
    in
      builtins.deepSeq
        (toolbeltLib.nexus.mkProfilesForHost {
          hostName = "client";
          deviceType = "server";
          hostToggles.bad = {
            description = "Bad profile";
            enable = true;
            deviceTypes = [ "laptop" ];
            specialisations.bad-off.enable = false;
          };
        })
        true
  '';

  badEvalResult =
    builtins.tryEval
    (builtins.deepSeq
      (toolbeltLib.nexus.mkProfilesForHost {
        hostName = "client";
        deviceType = "server";
        hostToggles.bad = {
          description = "Bad profile";
          enable = true;
          deviceTypes = ["laptop"];
          specialisations.bad-off.enable = false;
        };
      })
      true);

  emptyModule = toolbeltLib.nexus.mkProfilesForHost {
    hostName = "client";
    deviceType = "laptop";
    hostToggles = {};
  };

  transitionScript = pkgs.writeText "profile-transition-check.sh" (toolbeltLib.profiles.mkTransitionCheck {
    profile = "docked";
    fromEnabled = true;
    toEnabled = false;
    currentSystemRoot = "$PWD/current";
    body = ''
      [ "$current" = true ]
      [ "$next" = false ]
      touch "$PWD/transition-ran"
    '';
  });
in {
  nexus-profiles = pkgs.testers.nixosTest {
    name = "nexus-profiles";

    nodes.machine = {
      imports = [
        ({lib, ...}: {
          options.home-manager.sharedModules = lib.mkOption {
            type = lib.types.listOf lib.types.deferredModule;
            default = [];
            description = "Test-only stub for modules that mirror profile state into Home Manager.";
          };
        })
        ../modules/nixos/profiles.nix
        goodModule
      ];

      system.preSwitchChecks.bar-enter = toolbeltLib.nexus.mkProfileTransitionCheck {
        profile = "bar";
        from = false;
        to = true;
        script = ''
          printf 'entered\n' >> /var/lib/bar-enter-count
        '';
      };

      system.stateVersion = "25.11";
    };

    testScript = ''
      start_all()

      machine.succeed("test -e /etc/canix-profiles/foo.enable")
      machine.succeed("test -e /etc/canix-profiles/bar.enable")
      machine.succeed("grep -qx true /etc/canix-profiles/foo.enable")
      machine.succeed("grep -qx false /etc/canix-profiles/bar.enable")
      machine.succeed("test -e /run/current-system/specialisation/foo-off")
      machine.succeed("test -e /run/current-system/specialisation/bar-on")
      machine.succeed("readlink -f /run/current-system > /tmp/base-system")
      machine.succeed("/run/current-system/specialisation/bar-on/bin/switch-to-configuration switch")
      machine.succeed("test $(wc -l < /var/lib/bar-enter-count) -eq 1")
      machine.succeed("/run/current-system/bin/switch-to-configuration switch")
      machine.succeed("test $(wc -l < /var/lib/bar-enter-count) -eq 1")
      machine.succeed("$(cat /tmp/base-system)/specialisation/bar-off/bin/switch-to-configuration switch")
      machine.succeed("$(cat /tmp/base-system)/specialisation/bar-on/bin/switch-to-configuration switch")
      machine.succeed("test $(wc -l < /var/lib/bar-enter-count) -eq 2")
    '';
  };

  nexus-device-types-assertion = mkEvalCheck {
    name = "nexus-device-types-assertion";
    resultMessage = "deviceTypes assertion failed as expected";
    assertions = [
      {
        name = "device-types-assertion-failed";
        assertion = !badEvalResult.success;
        message = "expected nexus deviceTypes assertion to fail, but eval succeeded";
      }
      {
        name = "empty-input-handled";
        assertion = emptyModule == {canix-toolbelt.profiles = {};};
        message = "expected empty hostToggles to produce an empty profiles module";
      }
    ];
    runtimeScript = ''
      printf '%s\n' ${lib.escapeShellArg (toString badEval)} > "$out/bad-eval-path"
    '';
  };

  profile-transition-check =
    pkgs.runCommand "profile-transition-check" {
      inherit transitionScript;
      nativeBuildInputs = [pkgs.bash];
    } ''
      set -euo pipefail
      mkdir -p "$PWD/current/etc/canix-profiles" "$PWD/incoming/etc/canix-profiles"
      printf 'true\n' > "$PWD/current/etc/canix-profiles/docked.enable"
      printf 'false\n' > "$PWD/incoming/etc/canix-profiles/docked.enable"
      bash "$transitionScript" "$PWD/incoming" switch
      test -e "$PWD/transition-ran"
      rm "$PWD/transition-ran"

      printf 'false\n' > "$PWD/current/etc/canix-profiles/docked.enable"
      bash "$transitionScript" "$PWD/incoming" switch
      test ! -e "$PWD/transition-ran"
      touch "$out"
    '';
}
