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
    if ! unshare --user --map-root-user --pid --fork --mount-proc --kill-child=KILL --forward-signals -- ${pkgs.runtimeShell} -c 'exit 0'; then
      id
      cat /proc/self/uid_map /proc/self/gid_map
      if test -r /proc/self/attr/current; then cat /proc/self/attr/current; fi
      if test -r /proc/sys/kernel/apparmor_restrict_unprivileged_userns; then
        cat /proc/sys/kernel/apparmor_restrict_unprivileged_userns
      fi
      exit 1
    fi
  '';
  # Pkl's HTTP client initializes even for local-only manifest fixtures.
  SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
  meta.mainProgram = "canix-toolbelt";
}
