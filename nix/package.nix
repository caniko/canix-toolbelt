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
  # nixpkgs' cargo setup hook sets AWS_LC_SYS_USE_SYSTEM=1, so aws-lc-sys links this.
  buildInputs = [pkgs.aws-lc];
  nativeCheckInputs = [pkgs.git];
  # Pkl's HTTP client initializes even for local-only manifest fixtures.
  SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
  meta.mainProgram = "canix-toolbelt";
}
