{
  config,
  lib,
  ...
}: let
  profiles = config.canix-toolbelt.profiles;
  active = lib.filterAttrs (_: entry: entry.enable) config.canix-toolbelt.services.executorAdmission;
in {
  imports = [../profiles.nix ../activation-contracts.nix];

  options.canix-toolbelt.services.executorAdmission = lib.mkOption {
    default = {};
    description = "Live admission files for independently enabled executors. Profile changes drain new work without changing worker units.";
    type = lib.types.attrsOf (lib.types.submodule ({
      name,
      config,
      ...
    }: {
      options = {
        enable = lib.mkEnableOption "the ${name} executor admission policy";
        profile = lib.mkOption {
          type = lib.types.nullOr lib.types.str;
          default = null;
          description = "Optional toolbelt profile whose live enable value admits new runs.";
        };
        accepting = lib.mkOption {
          type = lib.types.bool;
          default = config.profile == null || (profiles.${config.profile}.enable or false);
          defaultText = lib.literalExpression "profile == null || profiles.<profile>.enable";
          description = "Whether new work may be admitted. This does not revoke existing run or Stop ownership.";
        };
        fileName = lib.mkOption {
          type = lib.types.strMatching "[A-Za-z0-9][A-Za-z0-9._-]*\\.json";
          default = "executor-${name}-admission.json";
          description = "Basename of the root-owned /etc policy file; it contains no credentials.";
        };
        file = lib.mkOption {
          type = lib.types.str;
          readOnly = true;
          default = "/etc/${config.fileName}";
          defaultText = lib.literalExpression ''"/etc/" + fileName'';
          description = "Stable path for the executor to reread before every new reservation.";
        };
        contract = lib.mkOption {
          type = lib.types.str;
          default = name;
          description = "Existing enabled activation contract to which this generated file belongs.";
        };
      };
    }));
  };

  config = {
    assertions =
      lib.concatLists (lib.mapAttrsToList (name: entry: [
          {
            assertion = entry.profile == null || builtins.hasAttr entry.profile profiles;
            message = "Executor admission ${name} references an undeclared toolbelt profile.";
          }
          {
            assertion = config.canix-toolbelt.activation.contracts.${entry.contract}.enabled;
            message = "Executor admission ${name} requires an enabled owning activation contract.";
          }
        ])
        active)
      ++ [
        {
          assertion = let
            names = map (entry: entry.fileName) (builtins.attrValues active);
          in
            builtins.length names == builtins.length (lib.unique names);
          message = "Executor admission policies must have unique /etc file names.";
        }
      ];
    environment.etc = lib.mapAttrs' (_: entry:
      lib.nameValuePair entry.fileName {
        text = builtins.toJSON {
          version = 1;
          inherit (entry) accepting;
        };
        mode = "0444";
      })
    active;
    # No unit dependencies or restart triggers reference the changing file.
    # The executor owns the request-time read and idempotent drain semantics.
    canix-toolbelt.activation.contracts = lib.mkMerge (lib.mapAttrsToList (_: entry: {
        ${entry.contract}.artifacts = [
          {
            kind = "file";
            path = entry.file;
            sensitive = false;
            persistence = "generated";
          }
        ];
      })
      active);
  };
}
