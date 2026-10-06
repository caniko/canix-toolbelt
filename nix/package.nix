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
  meta.mainProgram = "canix-toolbelt";
}
