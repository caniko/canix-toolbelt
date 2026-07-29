{
  lib,
  wrapper-manager,
}: let
  ozoneFlags = ozonePlatform:
    if ozonePlatform == "auto"
    then ["--ozone-platform-hint=auto"]
    else if lib.elem ozonePlatform ["wayland" "x11"]
    then [
      "--enable-features=UseOzonePlatform"
      "--ozone-platform=${ozonePlatform}"
    ]
    else throw "canix-toolbelt.chromiumGpu: ozonePlatform must be one of auto, wayland, or x11";

  mkWrapper = {
    pkgs,
    basePackage,
    wrapperName ? lib.getName basePackage,
    skipPrograms ? [],
    ozonePlatform ? "auto",
    extraFlags ? [],
    extraEnv ? {},
  }: let
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
      ((wrapper-manager.lib {
          inherit pkgs;
          modules = [
            {
              wrappers.${wrapperName} = {
                basePackage = wrapperBasePackage;
                prependFlags = ozoneFlags ozonePlatform ++ ["--use-gl=angle"] ++ extraFlags;
                env = extraEnv;
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
            inherit pkgs wrapperName skipPrograms ozonePlatform extraFlags extraEnv;
            basePackage = basePackage.override args;
          })
        (lib.functionArgs basePackage.override);
    };
in
  mkWrapper
