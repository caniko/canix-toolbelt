{
  lib,
  stdenv,
  rustc,
  zellij,
  pinentry-tty,
  pinentry-qt,
  systemd,
  python3,
  gnupg,
  rage,
  bash,
  xorg,
  xdotool,
}:
assert lib.versionAtLeast zellij.version "0.45";
  stdenv.mkDerivation {
    pname = "canix-toolbelt-pinentry";
    version = "0.2.0";
    src = lib.fileset.toSource {
      root = ../runtime/pinentry;
      fileset = lib.fileset.unions [../runtime/pinentry/main.rs ../runtime/pinentry/tests.rs ../runtime/pinentry/test.py ../runtime/pinentry/fixture_plugin.py];
    };
    nativeBuildInputs = [rustc];
    env = {
      CANIX_PINENTRY_ZELLIJ = lib.getExe zellij;
      CANIX_PINENTRY_TTY = lib.getExe pinentry-tty;
      CANIX_PINENTRY_QT = lib.getExe pinentry-qt;
      CANIX_PINENTRY_SYSTEMCTL = lib.getExe' systemd "systemctl";
      CANIX_PINENTRY_GPG = "${gnupg}/bin/gpg";
    };
    buildPhase = ''
      runHook preBuild
      rustc --edition=2024 -D warnings -C opt-level=2 main.rs -o canix-toolbelt-pinentry
      runHook postBuild
    '';
    doCheck = true;
    checkPhase = ''
      runHook preCheck
      rustc --edition=2024 -D warnings --test main.rs -o pinentry-tests
      ./pinentry-tests
      runHook postCheck
    '';
    installPhase = ''
      runHook preInstall
      install -Dm755 canix-toolbelt-pinentry "$out/bin/canix-toolbelt-pinentry"
      ln -s canix-toolbelt-pinentry "$out/bin/canix-toolbelt-pinentry-agent"
      ln -s canix-toolbelt-pinentry "$out/bin/pinentry"
      runHook postInstall
    '';
    doInstallCheck = true;
    nativeInstallCheckInputs = [python3 gnupg zellij bash rage xorg.xorgserver xdotool];
    installCheckPhase = ''
      runHook preInstallCheck
      ${lib.getExe python3} test.py "$out/bin/canix-toolbelt-pinentry-agent" \
        ${lib.getExe zellij} ${gnupg}/bin/gpg-connect-agent \
        ${gnupg}/bin/gpgconf ${lib.getExe bash} ${gnupg}/bin/gpg \
        "$out/bin/canix-toolbelt-pinentry" ${lib.getExe rage} \
        ${xorg.xorgserver}/bin/Xvfb ${lib.getExe xdotool}
      runHook postInstallCheck
    '';
    meta = {
      description = "Request-local Zellij, Qt and terminal pinentry for GPG and age clients";
      license = lib.licenses.mit;
      mainProgram = "canix-toolbelt-pinentry";
      platforms = lib.platforms.linux;
    };
  }
