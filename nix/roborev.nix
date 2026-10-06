{
  pkgs,
  harborGo,
  harborJs,
  buildPkgs ? pkgs.buildPackages,
}: let
  inherit (pkgs) lib;
  version = "0.71.0";
  src = pkgs.fetchFromGitHub {
    owner = "kenn-io";
    repo = "roborev";
    rev = "2087d5abc7a65f2d6de555ae49fc55795aefb78f";
    hash = "sha256-Lc0GVDKCfZDBo44SfEi2QnSikkzumtcoKOGXQupRYE8=";
  };
  toolchain = harborGo.lib.mkGoToolchain {
    inherit pkgs;
    # buildGoModule inherits target GOOS/GOARCH from this compiler attribute set.
    # Execute the native compiler while retaining the requested output target.
    go = buildPkgs.go_1_27 // {inherit (pkgs.stdenv.hostPlatform.go) GOOS GOARCH;};
  };
  bunToolchain = harborJs.lib.mkBunToolchain {
    pkgs = buildPkgs;
    # Match the checked-in root/web manifests, rather than CI's different Bun.
    version = "1.3.14";
  };
  deps = harborJs.lib.mkBunWorkspaceDeps {
    pkgs = buildPkgs;
    inherit src;
    inherit (bunToolchain) bun;
    packageJson = "${src}/package.json";
    lockfile = "${src}/bun.lock";
    pname = "roborev-bun-deps";
    # Avoid reading a fetch derivation's manifest during evaluation (IFD).
    inherit (bunToolchain) version;
    hash = "sha256-uVtflYyGY6twbL3h0AGCh81AeMTM7ETQMfwIaNwLHg0=";
    # Bun 1.3.14 sometimes omits these two locked peer-bin links in the
    # sandbox. Canonicalize this workspace to the already accepted artifact;
    # all dependency contents and the fixed output hash remain unchanged.
    postInstallNormalize = ''
      for peer in \
        node_modules/.bun/@eslint-community+eslint-utils@4.10.1+6d5dcfc5e09e405e/node_modules \
        node_modules/.bun/@eslint-community+eslint-utils@4.10.1+ce0cb4397c8174c7/node_modules; do
        test -f "$peer/eslint/bin/eslint.js"
        mkdir -p "$peer/.bin"
        ln -sfn ../eslint/bin/eslint.js "$peer/.bin/eslint"
      done
    '';
  };
  frontend = buildPkgs.stdenvNoCC.mkDerivation {
    pname = "roborev-web";
    inherit version src;
    nativeBuildInputs = [bunToolchain.bun buildPkgs.nodejs];
    configurePhase = ''
      runHook preConfigure
      export HOME="$TMPDIR/home" BUN_INSTALL_CACHE_DIR="$TMPDIR/bun-cache"
      mkdir -p "$HOME" "$BUN_INSTALL_CACHE_DIR"
      cp -R ${deps}/. .
      chmod -R u+w node_modules web/node_modules packages
      patchShebangs node_modules web/node_modules
      runHook postConfigure
    '';
    buildPhase = ''
      runHook preBuild
      bun run --cwd web generate:check
      bun run --cwd web typecheck
      bun run --cwd web build
      bun run --cwd web assets:check
      runHook postBuild
    '';
    installPhase = ''
      runHook preInstall
      mkdir -p "$out"
      cp -R web/dist/. "$out/"
      test -s "$out/.vite/manifest.json"
      runHook postInstall
    '';
  };
  package = harborGo.lib.mkGoPackage {
    inherit pkgs toolchain version src;
    pname = "roborev";
    vendorHash = "sha256-deCvjJHN3uNK+KYfgoJKxiZFShW5bVnLMksTx2SdrS4=";
    subPackages = ["cmd/roborev"];
    # Agent/config tests create isolated Git fixtures inside the sandbox.
    nativeBuildInputs = [buildPkgs.git];
    # The ACP test assets use /usr/bin/env, absent in the Nix sandbox.
    postPatch = ''
      patchShebangs scripts/acp-agent scripts/acp-agent-codex scripts/acp-agent-claude scripts/acp-agent-gemini
    '';
    env.CGO_ENABLED = "0";
    ldflags = ["-s" "-w" "-X go.kenn.io/roborev/internal/version.Version=v${version}"];
    # buildGoModule does not inherit this hook into the dependency derivation.
    postConfigure = ''
      rm -rf internal/web/dist
      mkdir -p internal/web/dist
      cp -R ${frontend}/. internal/web/dist/
      chmod -R u+w internal/web/dist
    '';
    doCheck = pkgs.stdenv.buildPlatform.canExecute pkgs.stdenv.hostPlatform;
    checkPhase = ''
      runHook preCheck
      export HOME="$TMPDIR/check-home" ROBOREV_DATA_DIR="$TMPDIR/check-state"
      export ROBOREV_TELEMETRY_ENABLED=0 ROBOREV_RUN_WEB_RELEASE_CHECK=1
      mkdir -p "$HOME"
      # ACP tests locate checked-in scripts through runtime.Caller. Override
      # buildGoModule's trimpath only for test binaries, not the release output.
      if ! GOMAXPROCS=2 go test -trimpath=false -p 2 ./internal/config ./internal/web ./internal/agent > "$TMPDIR/roborev-tests.log" 2>&1; then
        # Keep causal failures visible in the bounded Canix build-error tail.
        grep -A 12 -- '--- FAIL:' "$TMPDIR/roborev-tests.log" || cat "$TMPDIR/roborev-tests.log"
        exit 1
      fi
      cat "$TMPDIR/roborev-tests.log"
      runHook postCheck
    '';
    doInstallCheck = pkgs.stdenv.buildPlatform.canExecute pkgs.stdenv.hostPlatform;
    installCheckPhase = ''
      runHook preInstallCheck
      export HOME="$TMPDIR/install-home" ROBOREV_DATA_DIR="$TMPDIR/roborev"
      export ROBOREV_TELEMETRY_ENABLED=0
      mkdir -p "$HOME"
      "$out/bin/roborev" verify-web-assets
      "$out/bin/roborev" version
      runHook postInstallCheck
    '';
    passthru = {inherit frontend deps toolchain bunToolchain;};
    meta = {
      description = "Continuous code review orchestrator with embedded web assets";
      homepage = "https://github.com/kenn-io/roborev";
      license = lib.licenses.mit;
      mainProgram = "roborev";
      platforms = ["x86_64-linux" "aarch64-linux"];
    };
  };
in
  assert lib.assertMsg (toolchain.go.version == "1.27.1")
  "roborev's frozen Harbor contract requires Go 1.27.1; requalify before changing toolchains";
  assert lib.assertMsg
  # The pinned stdenv flattens its input `env` onto the resulting derivation.
  (package.GOOS == pkgs.stdenv.hostPlatform.go.GOOS && package.GOARCH == pkgs.stdenv.hostPlatform.go.GOARCH)
  "roborev Go output metadata must match its target platform"; package
