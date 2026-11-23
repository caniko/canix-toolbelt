{
  lib,
  wrapper-manager,
}: let
  ozoneFlags = ozonePlatform:
    if ozonePlatform == "auto"
    then ["--ozone-platform-hint=auto"]
    else if lib.elem ozonePlatform ["wayland" "x11"]
    then ["--ozone-platform=${ozonePlatform}"]
    else throw "canix-toolbelt.chromiumGpu: ozonePlatform must be one of auto, wayland, or x11";

  # Select the VA-API device and driver without forcing Chromium features.
  mediaPolicy = gpuMedia: let
    enabled = gpuMedia != null && (gpuMedia.enable or false);
  in
    if !enabled
    then {
      flags = [];
      env = {};
    }
    else let
      isNvidia = gpuMedia.vendor == "nvidia";
    in {
      flags = ["--render-node-override=${gpuMedia.renderNode}"];
      env =
        {
          LIBVA_DRIVER_NAME = {
            value = gpuMedia.libvaDriver;
            force = true;
          };
        }
        // lib.optionalAttrs isNvidia {NVD_BACKEND.value = "direct";};
    };

  mkWrapper = {
    pkgs,
    basePackage,
    wrapperName ? lib.getName basePackage,
    skipPrograms ? [],
    ozonePlatform ? "auto",
    gpuMedia ? null,
    extraFlags ? [],
    extraFeatures ? [],
    extraEnv ? {},
  }: let
    media = mediaPolicy gpuMedia;
    flags =
      ozoneFlags ozonePlatform
      ++ media.flags
      ++ lib.optional (extraFeatures != []) "--enable-features=${lib.concatStringsSep "," extraFeatures}"
      ++ extraFlags;
    env = media.env // extraEnv;
    wrapperBasePackage =
      if basePackage ? overrideAttrs
      then
        basePackage.overrideAttrs (old: {
          # wrapper-manager copies basePackage.meta onto the generated
          # symlinkJoin. Keep license enforcement on the real package, not
          # on the local wrapper derivation.
          meta = removeAttrs (old.meta or {}) ["license" "sourceProvenance"];
        })
      else basePackage;
    wrapped =
      (wrapper-manager.lib {
        inherit pkgs;
        modules = [
          {
            wrappers.${wrapperName} = {
              basePackage = wrapperBasePackage;
              prependFlags = flags;
              inherit env;
              programs = lib.genAttrs skipPrograms (_: {});
            };
          }
        ];
      }).config.wrappers.${
        wrapperName
      }
          .wrapped;
  in
    wrapped
    // {
      pname = basePackage.pname or basePackage.name;
      version = basePackage.version or "";
      meta = basePackage.meta or {};
      passthru = basePackage.passthru or {};
    }
    // lib.optionalAttrs (basePackage ? override) {
      override =
        lib.setFunctionArgs
        (args:
          mkWrapper {
            inherit pkgs wrapperName skipPrograms ozonePlatform gpuMedia extraFlags extraFeatures extraEnv;
            basePackage = basePackage.override args;
          })
        (lib.functionArgs basePackage.override);
    };
in
  mkWrapper
