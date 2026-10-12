{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.programs.roborev;
  toml = pkgs.formats.toml {};
  schema = import ./schema.nix {inherit lib toml;};
in {
  options.programs.roborev.repositories = lib.mkOption {
    default = {};
    description = "Named per-repository configuration artifacts. Rendering does not modify or enroll a checkout; explicitly select configFile in the checkout's own configuration/deployment mechanism.";
    type = lib.types.attrsOf (lib.types.submodule ({
      name,
      config,
      ...
    }: {
      options = {
        settings = lib.mkOption {
          type = schema.repositoryType;
          default = {};
          description = "Per-repository upstream TOML with nullable inheritance and a freeform escape hatch.";
        };
        configFile = lib.mkOption {
          type = lib.types.package;
          readOnly = true;
          description = "Rendered .roborev.toml artifact; no implicit checkout write or hook installation.";
        };
        target = lib.mkOption {
          type = lib.types.nullOr lib.types.str;
          default = null;
          description = "Explicit Home Manager-owned .roborev.toml destination below homeDirectory. Null renders only. Existing unmanaged files cause activation to fail; no force, enrollment or Git staging is performed.";
        };
      };
      config.configFile = toml.generate "roborev-repository-${builtins.substring 0 16 (builtins.hashString "sha256" name)}.toml" (schema.clean config.settings);
    }));
  };
  config = lib.mkIf cfg.enable {
    home.file = lib.mapAttrs' (name: repository:
      lib.nameValuePair "roborev-repository-${name}" {
        inherit (repository) target;
        source = repository.configFile;
      }) (lib.filterAttrs (_: repository: repository.target != null) cfg.repositories);
    assertions = lib.concatLists (lib.mapAttrsToList (name: repository: let
      settings = schema.clean repository.settings;
    in
      [
        {
          assertion = builtins.all (key: !(builtins.hasAttr key settings)) schema.globalOnly;
          message = "roborev.repositories.${name}.settings contains a global-only configuration key.";
        }
        {
          assertion = lib.attrByPath ["ci" "github_app_private_key"] "" settings == "";
          message = "Repository configuration cannot contain GitHub App credentials.";
        }
        {
          assertion =
            repository.target
            == null
            || (
              lib.hasPrefix "${config.home.homeDirectory}/" repository.target
              && lib.hasSuffix "/.roborev.toml" repository.target
              && !lib.hasInfix "/../" repository.target
              && !lib.hasInfix "/./" repository.target
              && !lib.hasInfix "//" repository.target
              && builtins.match ".*[[:cntrl:]].*" repository.target == null
            );
          message = "roborev.repositories.${name}.target must be a normalized .roborev.toml path below homeDirectory.";
        }
      ]
      ++ (import ./credentials.nix {
        inherit lib settings;
        # Global credentials are forbidden above; repository hooks use private
        # externally provisioned configuration instead of a store artifact.
        runtimeSecretPath = _: false;
        envReference = _: false;
      }))
    cfg.repositories);
  };
}
