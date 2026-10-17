{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.programs.roborev;
  profiles = ["claude" "codex" "copilot" "cursor" "droid" "gemini" "hermes" "qwen" "grok"];
  sharedProfiles = ["gemini" "hermes" "qwen"];
  hookFormat = name:
    if name == "hermes"
    then pkgs.formats.yaml {}
    else pkgs.formats.json {};
  mcpFormat = name:
    if builtins.elem name ["codex" "grok"]
    then pkgs.formats.toml {}
    else hookFormat name;
  targetOption = description:
    lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      inherit description;
    };
  validTarget = value:
    value
    == null
    || (
      lib.hasPrefix "${config.home.homeDirectory}/" value
      && !lib.hasInfix "/../" "${value}/"
      && !lib.hasInfix "/./" "${value}/"
      && !lib.hasInfix "//" value
      && !lib.hasSuffix "/" value
      && builtins.match ".*[[:cntrl:]].*" value == null
    );
  selected = lib.filterAttrs (_: integration: integration.hooks.enable || integration.mcp.enable || integration.skills.enable) cfg.integrations;
  artifact = name: integration: let
    shared = builtins.elem name sharedProfiles && integration.hooks.enable && integration.mcp.enable;
    hookBase = (hookFormat name).generate "roborev-${name}-hook-base" integration.hooks.settings;
    mcpBase = (mcpFormat name).generate "roborev-${name}-mcp-base" integration.mcp.settings;
    hookCommand = lib.escapeShellArgs ["${cfg.finalPackage}/bin/roborev" "agent-hook" "run" "--agent" name];
    serverFlags = lib.optionals (integration.server != "") ["--roborev-server" integration.server];
  in
    pkgs.runCommand "roborev-${name}-integration" {nativeBuildInputs = [pkgs.coreutils pkgs.git];} ''
      export HOME="$TMPDIR/home"
      mkdir -p "$HOME" "$out"
      export ROBOREV_DATA_DIR="$HOME/.roborev" ROBOREV_TELEMETRY_ENABLED=0
      ${lib.optionalString integration.hooks.enable ''
        cp ${hookBase} "$TMPDIR/hooks"
        chmod u+w "$TMPDIR/hooks"
        ${cfg.rendererPackage}/bin/roborev agent-hook dump ${lib.escapeShellArgs ["--agent" name "--command" hookCommand "--timeout" "${toString integration.hooks.timeoutSeconds}s" "--config"]} "$TMPDIR/hooks" ${lib.escapeShellArgs (serverFlags ++ lib.optionals integration.mcp.enable (["--mcp" "--mcp-transport" integration.mcp.transport] ++ lib.optionals (integration.mcp.url != null) ["--mcp-url" integration.mcp.url]))} > "$out/hooks"
      ''}
      ${lib.optionalString integration.mcp.enable ''
        cp ${
          if shared
          then ''"$out/hooks"''
          else toString mcpBase
        } "$out/mcp"
        chmod u+w "$out/mcp"
        ${cfg.rendererPackage}/bin/roborev ${lib.escapeShellArgs (lib.optionals (integration.server != "") ["--server" integration.server])} mcp install \
          ${lib.escapeShellArgs (["--agent" name "--binary" "${cfg.finalPackage}/bin/roborev" "--transport" integration.mcp.transport]
            ++ lib.optionals (integration.mcp.url != null) ["--url" integration.mcp.url])} --config "$out/mcp"
        ${lib.optionalString shared ''cp "$out/mcp" "$out/hooks"''}
      ''}
      ${lib.optionalString integration.skills.enable ''
        ${cfg.rendererPackage}/bin/roborev skills install --agent ${lib.escapeShellArg name} --path "$out/skills" --mcp=${lib.boolToString integration.mcp.enable}
      ''}
    '';
in {
  options.programs.roborev.rendererPackage = lib.mkOption {
    type = lib.types.package;
    default = cfg.package;
    defaultText = lib.literalExpression "config.programs.roborev.package";
    description = "Build-platform Roborev executable for native harness rendering. Cross consumers must supply the same pinned Roborev version built for the builder's platform.";
  };
  options.programs.roborev.integrations = lib.mkOption {
    default = {};
    description = "Opt-in native harness artifacts from the pinned Roborev renderer. Profile names differ from review agents; OpenCode has no upstream native hook profile. Targets are explicit and unmanaged collisions fail activation.";
    type = lib.types.attrsOf (lib.types.submodule ({
      name,
      config,
      ...
    }: {
      options = {
        package = lib.mkOption {
          type = lib.types.package;
          readOnly = true;
          description = "Rendered hooks, MCP configuration and skills directory.";
        };
        server = lib.mkOption {
          type = lib.types.str;
          default = "";
          description = "Explicit daemon address for hooks and stdio MCP. Empty uses discovery.";
        };
        hooks = {
          enable = lib.mkEnableOption "native Roborev agent hooks";
          timeoutSeconds = lib.mkOption {
            type = lib.types.ints.unsigned;
            default = 10;
            description = "Native hook timeout in seconds; zero omits the timeout.";
          };
          settings = lib.mkOption {
            inherit ((hookFormat name)) type;
            default = {};
            description = "Non-secret base native harness configuration, merged by upstream's hook renderer.";
          };
          target = targetOption "Optional Home Manager-owned hook configuration destination. Null renders only.";
        };
        mcp = {
          enable = lib.mkEnableOption "a native Roborev MCP client entry";
          transport = lib.mkOption {
            type = lib.types.enum ["stdio" "http"];
            default = "stdio";
            description = "MCP client transport; HTTP also requires an existing daemon /mcp URL.";
          };
          url = targetOption "Existing HTTP daemon endpoint ending in /mcp. Never include credentials in this URL.";
          settings = lib.mkOption {
            inherit ((mcpFormat name)) type;
            default = {};
            description = "Non-secret base MCP-client configuration. With shared native hook files, put base settings in hooks.settings instead.";
          };
          target = targetOption "Optional Home Manager-owned MCP-client configuration destination. Shared hook/MCP formats can use the same target.";
        };
        skills = {
          enable = lib.mkEnableOption "the upstream Roborev skill bundle";
          target = targetOption "Optional Home Manager-owned final skills directory. Null renders only; select a dedicated directory to avoid taking over unrelated skills.";
        };
      };
      config.package = artifact name config;
    }));
  };
  config = lib.mkIf cfg.enable {
    assertions = lib.concatLists (lib.mapAttrsToList (name: integration: [
        {
          assertion = builtins.elem name profiles;
          message = "roborev.integrations.${name}: unsupported upstream harness profile.";
        }
        {
          assertion = builtins.all validTarget [integration.hooks.target integration.mcp.target integration.skills.target];
          message = "Roborev integration targets must be normalized paths below homeDirectory.";
        }
        {
          assertion = integration.hooks.target == null || integration.hooks.enable;
          message = "Roborev hook target requires hooks.enable.";
        }
        {
          assertion = integration.mcp.target == null || integration.mcp.enable;
          message = "Roborev MCP target requires mcp.enable.";
        }
        {
          assertion = integration.skills.target == null || integration.skills.enable;
          message = "Roborev skills target requires skills.enable.";
        }
        {
          assertion =
            !integration.mcp.enable
            || (
              if integration.mcp.transport == "http"
              then integration.mcp.url != null && builtins.match "https?://[^/@?#]+/mcp" integration.mcp.url != null
              else integration.mcp.url == null
            );
          message = "Roborev HTTP MCP requires a credential-free /mcp URL; stdio must omit url.";
        }
        {
          assertion = !integration.hooks.enable || !integration.mcp.enable || integration.mcp.transport != "http" || integration.server == "";
          message = "HTTP MCP selects the native hook daemon; omit integration.server.";
        }
        {
          assertion = !(builtins.elem name sharedProfiles && integration.hooks.enable && integration.mcp.enable) || integration.mcp.settings == {};
          message = "Shared native hook/MCP files require base configuration in hooks.settings only.";
        }
        {
          assertion = integration.hooks.target == null || integration.hooks.target != integration.mcp.target || builtins.elem name sharedProfiles;
          message = "This harness requires separate native hook and MCP files.";
        }
      ])
      cfg.integrations);
    home.file = lib.listToAttrs (lib.concatLists (lib.mapAttrsToList (
        name: integration:
          lib.optional (integration.hooks.target != null) {
            name = "roborev-${name}-hooks";
            value = {
              target = integration.hooks.target;
              source = "${integration.package}/hooks";
            };
          }
          ++ lib.optional (integration.mcp.target != null && integration.mcp.target != integration.hooks.target) {
            name = "roborev-${name}-mcp";
            value = {
              target = integration.mcp.target;
              source = "${integration.package}/mcp";
            };
          }
          ++ lib.optional (integration.skills.target != null) {
            name = "roborev-${name}-skills";
            value = {
              target = integration.skills.target;
              source = "${integration.package}/skills";
              recursive = true;
            };
          }
      )
      selected));
  };
}
