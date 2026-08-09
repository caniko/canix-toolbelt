{
  inputs,
  pkgs,
  ...
}: let
  hm = inputs.home-manager.lib;
  gooseInput = inputs.canix-toolbelt or inputs.self;
  defaultGooseModule = gooseInput.homeModules.goose;
  mkGooseHome = {
    extraModules ? [],
    hostname ? "atlas",
    gooseModule ? defaultGooseModule,
    preModules ? [],
  }:
    hm.homeManagerConfiguration {
      inherit pkgs;

      extraSpecialArgs = {
        inherit hostname inputs;
      };

      modules =
        preModules
        ++ [
          gooseModule
          {
            home = {
              username = "goose-test";
              homeDirectory = "/home/goose-test";
              stateVersion = "25.11";
            };

            programs.home-manager.enable = true;
            manual = {
              html.enable = false;
              json.enable = false;
              manpages.enable = false;
            };
          }
        ]
        ++ extraModules;
    };
in {
  mkGooseHome = {
    extraModules ? [],
    hostname ? "atlas",
    gooseModule ? defaultGooseModule,
    preModules ? [],
  }:
    hm.homeManagerConfiguration {
      inherit pkgs;

      extraSpecialArgs = {
        inherit hostname inputs;
      };

      modules =
        preModules
        ++ [
          gooseModule
          {
            home = {
              username = "goose-test";
              homeDirectory = "/home/goose-test";
              stateVersion = "25.11";
            };

            programs.home-manager.enable = true;
            manual = {
              html.enable = false;
              json.enable = false;
              manpages.enable = false;
            };
          }
        ]
        ++ extraModules;
    };

  mkFixture = home: {
    inherit home;
    inherit (home.config.programs.goose) generated;
    packagePaths = builtins.toJSON (
      map (pkg: builtins.unsafeDiscardStringContext pkg.outPath) home.config.home.packages
    );
  };

  mkGooseCheck = {
    name,
    fixture,
    body,
    nativeBuildInputs ? [
      pkgs.jq
      pkgs.yq-go
    ],
    extraAttrs ? {},
  }:
    pkgs.runCommand name ({
        configDir = toString fixture.generated.configDir;
        stateSpec = toString fixture.generated.stateSpec;
        stateScript = toString fixture.generated.stateScript;
        inherit nativeBuildInputs;
      }
      // extraAttrs) ''
      spec="$stateSpec"
      stateScript="$stateScript"
      config_dir="$configDir"

      ${body}

      touch $out
    '';

  mkExpectedEvalFailure = args: let
    normalizedArgs =
      if builtins.isList args
      then {extraModules = args;}
      else args;
  in
    builtins.tryEval (mkGooseHome normalizedArgs).activationPackage;

  mkExpectedEvalFailureCheck = {
    name,
    result,
  }:
    pkgs.runCommand name {
      success =
        if result.success
        then "1"
        else "0";
    } ''
      test "$success" = "0"
      touch $out
    '';
}
