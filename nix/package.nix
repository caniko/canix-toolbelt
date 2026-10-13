{
  pkgs,
  buildTrain ? false,
  directNetwork ? false,
}:
pkgs.rustPlatform.buildRustPackage {
  pname = "canix-toolbelt";
  version = (builtins.fromTOML (builtins.readFile ../Cargo.toml)).package.version;
  src = pkgs.lib.cleanSource ../.;
  cargoLock.lockFile = ../Cargo.lock;
  buildFeatures = ["cli"] ++ pkgs.lib.optional buildTrain "build-train" ++ pkgs.lib.optional directNetwork "direct-network";
  nativeBuildInputs = [pkgs.cmake pkgs.pkg-config pkgs.perl];
  nativeCheckInputs = [pkgs.git] ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [pkgs.util-linux];
  # Namespace admission is required even inside the packaging sandbox. Diagnose
  # failures before Cargo fixtures discard their failed execution directories.
  preCheck = pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux ''
    unshare --user --map-current-user --pid --fork --mount-proc --kill-child=KILL --forward-signals -- ${pkgs.runtimeShell} -c 'exit 0'
  '';
  # Pkl's HTTP client initializes even for local-only manifest fixtures.
  SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
  meta.mainProgram = "canix-toolbelt";
}
