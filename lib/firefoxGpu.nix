{
  lib,
  wrapper-manager,
}: let
  # Route Firefox-family video decode through the configured media GPU.
  # OpenGL/WebRender are left to Wayland; only decode needs a device + driver.
  mkWrapper = {
    pkgs,
    basePackage,
    wrapperName ? lib.getName basePackage,
    skipPrograms ? [],
    gpuMedia ? null,
    extraEnv ? {},
  }: let
    mediaEnv =
      if gpuMedia != null && (gpuMedia.enable or false)
      then {
        MOZ_DRM_DEVICE.value = gpuMedia.renderNode;
        LIBVA_DRIVER_NAME = {
          value = gpuMedia.libvaDriver;
          force = true;
        };
      }
      else {};
    env = mediaEnv // extraEnv;
    wrapperBasePackage =
      if basePackage ? overrideAttrs
      then
        basePackage.overrideAttrs (old: {
          meta = removeAttrs (old.meta or {}) ["license" "sourceProvenance"];
        })
      else basePackage;
    wrapped =
      ((wrapper-manager.lib {
          inherit pkgs;
          modules = [
            {
              wrappers.${wrapperName} = {
                basePackage = wrapperBasePackage;
                env = env;
                programs = lib.genAttrs skipPrograms (_: {});
              };
            }
          ];
        }).config.wrappers.${
          wrapperName
        })
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
            inherit pkgs wrapperName skipPrograms gpuMedia extraEnv;
            basePackage = basePackage.override args;
          })
        (lib.functionArgs basePackage.override);
    };
in
  mkWrapper
