{
  lib,
  buildNpmPackage,
  fetchurl,
  nodejs,
  makeWrapper,
  autoPatchelfHook,
  stdenv,
  claude-code,
}: let
  rawManifest = lib.importJSON ./claude-code-manifest.json;
  claude =
    if lib.versionOlder claude-code.version rawManifest.version
    then let
      # Older nixpkgs fetches /claude; newer recipes fetch and unzstd /claude.zst.
      # Match the existing recipe's transport, retaining its installation hooks.
      compressed = lib.hasSuffix ".zst" claude-code.src.url;
      manifest =
        if compressed
        then lib.importJSON ./claude-code-manifest.zst.json
        else rawManifest;
      binaries =
        if compressed
        then ["claude.zst" "claude.exe.zst"]
        else ["claude" "claude.exe"];
    in
      assert manifest.version == rawManifest.version;
      assert lib.assertMsg
      (lib.all (entry: lib.elem entry.binary binaries) (builtins.attrValues manifest.platforms))
      "canix-toolbelt: the Claude manifest must match the package's download format";
        claude-code.override {inherit manifest;}
    else claude-code;
in
  assert lib.versionAtLeast nodejs.version "22.15";
    buildNpmPackage (finalAttrs: {
      pname = "opencode-with-claude";
      version = "1.11.1";
      src = fetchurl {
        url = "https://registry.npmjs.org/opencode-with-claude/-/opencode-with-claude-${finalAttrs.version}.tgz";
        hash = "sha256-ZwW8D8oSnCegK65OHORhnANnDw8eYk7RJp5Ap2rTs84=";
      };
      sourceRoot = "package";
      npmDepsHash = "sha256-NN3sxBzlkbUeKAlDwV4L6nM6NjpB2kdy/fyJPHVCI+g=";
      dontNpmBuild = true;
      npmFlags = ["--ignore-scripts"];
      nativeBuildInputs = [makeWrapper autoPatchelfHook];
      buildInputs = [stdenv.cc.cc.lib];
      postPatch = ''
        cp ${./package.json} package.json
        cp ${./package-lock.json} package-lock.json
      '';
      postConfigure = ''
        node ${./patch-external.mjs} dist/index.js ${../../runtime/opencode-claude/external-runtime.mjs}
      '';
      postInstall = ''
        root="$out/lib/node_modules/opencode-with-claude"
        # The pinned V2 loader discovers local directories through server.js.
        printf '%s\n' 'export { default } from "./dist/index.js";' > "$root/server.js"
        # Claude is supplied by Nix; upstream's install scripts stay disabled.
        rm -rf "$root"/node_modules/@anthropic-ai/claude-code \
          "$root"/node_modules/@anthropic-ai/claude-code-* \
          "$root"/node_modules/@anthropic-ai/claude-agent-sdk-* \
          "$root"/node_modules/.bin/claude
        makeWrapper ${lib.getExe nodejs} "$out/bin/meridian" \
          --add-flags "$root/node_modules/@rynfar/meridian/dist/cli.js" \
          --set MERIDIAN_CLAUDE_PATH ${lib.getExe claude}
      '';
      doInstallCheck = true;
      installCheckPhase = ''
        runHook preInstallCheck
        OPENCODE_CLAUDE_PLUGIN="$out/lib/node_modules/opencode-with-claude/dist/index.js" \
          node --test ${../../runtime/opencode-claude/plugin.test.mjs}
        node --test ${../../runtime/opencode-claude}/external-runtime.test.mjs
        runHook postInstallCheck
      '';
      passthru.claudePackage = claude;
      meta = {
        description = "OpenCode V2 Claude subscription plugin and its pinned Meridian backend";
        homepage = "https://github.com/ianjwhite99/opencode-with-claude";
        license = lib.licenses.mit;
        platforms = lib.platforms.linux;
        mainProgram = "meridian";
      };
    })
