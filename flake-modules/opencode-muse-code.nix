{inputs, ...}: {
  perSystem = {
    pkgs,
    lib,
    system,
    ...
  }: let
    plugin = ../runtime/opencode-muse-code;
    legacyRuntime = import (plugin + "/package.nix") inputs.opencode.packages.${system}.opencode;
  in {
    checks =
      {
        opencode-muse-code-module = import ../nixos-tests/opencode-muse-code-eval.nix {inherit pkgs;};
        opencode-muse-code-protocol =
          pkgs.runCommand "opencode-muse-code-protocol" {
            nativeBuildInputs = [pkgs.nodejs];
          } ''
            node --test ${plugin}/protocol.test.mjs
            touch "$out"
          '';
        opencode-muse-code-v2 =
          pkgs.runCommand "opencode-muse-code-v2" {
            nativeBuildInputs = [pkgs.nodejs];
          } ''
            node --test ${plugin}/v2.test.mjs
            touch "$out"
          '';
      }
      // lib.optionalAttrs (inputs ? opencode) {
        muse-code-subscription =
          pkgs.runCommand "muse-code-subscription" {
            nativeBuildInputs = [pkgs.nodejs];
          } ''
            node ${legacyRuntime.node_modules}/packages/opencode/node_modules/typescript/bin/tsc \
              --noEmit --allowJs --checkJs --skipLibCheck --target es2023 --module nodenext \
              --types bun --typeRoots ${legacyRuntime.node_modules}/packages/opencode/node_modules/@types \
              ${plugin}/index.mjs ${plugin}/protocol.mjs
            node --test ${plugin}/protocol.test.mjs
            MUSE_TEST_OPENCODE=${legacyRuntime}/bin/opencode node --test ${plugin}/packaged.test.mjs
            touch "$out"
          '';
      };
    packages = lib.optionalAttrs (inputs ? opencode) {
      opencode-muse-code = legacyRuntime;
    };
  };
}
