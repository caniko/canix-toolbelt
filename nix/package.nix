{pkgs}:
pkgs.rustPlatform.buildRustPackage {
  pname = "canix-toolbelt";
  version = (builtins.fromTOML (builtins.readFile ../Cargo.toml)).package.version;
  src = pkgs.lib.cleanSource ../.;
  cargoLock.lockFile = ../Cargo.lock;
  buildFeatures = ["cli"];
  nativeBuildInputs = [pkgs.cmake pkgs.pkg-config pkgs.perl];
  meta.mainProgram = "canix-toolbelt";
}
