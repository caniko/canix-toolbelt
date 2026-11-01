{
  inputs,
  pkgs,
}: let
  inherit (import ./lib/eval-checks.nix {inherit pkgs;}) mkEvalCheck;
  inherit (inputs.nixpkgs) lib;

  eval = modules:
    lib.nixosSystem {
      inherit (pkgs.stdenv.hostPlatform) system;
      modules =
        [
          ../modules/nixos/hardware/gpu/intel/arc/common.nix
          {
            nixpkgs.config.allowUnfree = true;
            system.stateVersion = "25.11";
          }
        ]
        ++ modules;
    };

  # Stand-in for harbor-game's Steam options.
  stub = {
    options.harbor-game.steam = {
      extraArgs = lib.mkOption {
        type = lib.types.str;
        default = "";
      };
      extraEnv = lib.mkOption {
        type = lib.types.attrsOf lib.types.str;
        default = {};
      };
    };
  };
  steam = profile: (eval [stub {canix-toolbelt.hardware.gpu.profile = profile;}]).config.harbor-game.steam;
  xe = steam "intel-arc-a770-xe";
  i915 = steam "intel-arc-a770-i915";
  # Without the consumer module the quirk must be a no-op, not an error.
  bare = eval [{canix-toolbelt.hardware.gpu.profile = "intel-arc-a770-xe";}];
in
  mkEvalCheck {
    name = "gpu-steam-quirk-eval";
    assertions = [
      {
        name = "xe-quirk";
        assertion = xe.extraArgs == "-cef-disable-gpu" && xe.extraEnv == {LIBVA_DRIVER_NAME = "dummy";};
        message = "intel-arc-a770-xe must disable Steam CEF GPU and VA-API";
      }
      {
        name = "i915-untouched";
        assertion = i915.extraArgs == "" && i915.extraEnv == {};
        message = "only the xe profile gets the Steam quirk";
      }
      {
        name = "noop-without-harbor-game";
        assertion = !(bare.options ? harbor-game) && bare.config.hardware.graphics.enable;
        message = "module must evaluate unchanged without harbor-game";
      }
    ];
  }
