{
  pkgs,
  buildTrain ? false,
}:
pkgs.rustPlatform.buildRustPackage {
  pname = "canix-toolbelt";
  version = (builtins.fromTOML (builtins.readFile ../Cargo.toml)).package.version;
  src = pkgs.lib.cleanSource ../.;
  cargoLock.lockFile = ../Cargo.lock;
  buildFeatures = ["cli"] ++ pkgs.lib.optional buildTrain "build-train";
  nativeBuildInputs = [pkgs.cmake pkgs.pkg-config pkgs.perl];
  nativeCheckInputs = [pkgs.git];
  # Pkl's HTTP client initializes even for local-only manifest fixtures.
  SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
  meta.mainProgram = "canix-toolbelt";
}
