{pkgs, ...}: let
  toolbeltLib = import ../lib {inherit (pkgs) lib;};

  goodModule = toolbeltLib.nexus.mkProfilesForHost {
    hostName = "client";
    deviceType = "laptop";
    hostToggles = {
      foo = {
        description = "Foo profile";
        default = true;
        deviceTypes = ["laptop"];
        specialisations.foo-off.enable = false;
      };

      bar = {
        description = "Bar profile";
        default = false;
        specialisations.bar-on.enable = true;
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
            default = true;
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
          default = true;
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
    '';
  };

  nexus-device-types-assertion =
    pkgs.runCommand "nexus-device-types-assertion" {
      assertionFailed =
        if badEvalResult.success
        then throw "expected nexus deviceTypes assertion to fail, but eval succeeded"
        else "1";
      emptyInputHandled =
        if emptyModule == {canix-toolbelt.profiles = {};}
        then "1"
        else throw "expected empty hostToggles to produce an empty profiles module";
    } ''
      mkdir -p "$out"
      printf '%s\n' "deviceTypes assertion failed as expected" > "$out/result"
      printf '%s\n' ${badEval} > "$out/bad-eval-path"
    '';
}
