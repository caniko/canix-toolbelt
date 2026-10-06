{pkgs}: let
  root = ../crates/roborev-worker;
in
  pkgs.rustPlatform.buildRustPackage {
    pname = "canix-toolbelt-roborev-worker";
    version = (builtins.fromTOML (builtins.readFile (root + "/Cargo.toml"))).package.version;
    src = pkgs.lib.fileset.toSource {
      inherit root;
      fileset = pkgs.lib.fileset.unions [
        (root + "/Cargo.toml")
        (root + "/Cargo.lock")
        (root + "/README.md")
        (root + "/LICENSE")
        (root + "/src")
        (root + "/tests")
      ];
    };
    cargoLock.lockFile = root + "/Cargo.lock";
    # Native process/namespace qualification is a separate, explicitly selected
    # tier. Production builds run the default admission/parser tests only.
    buildFeatures = [];
    meta = {
      description = "Toolbelt's trusted Linux Roborev worker helper";
      mainProgram = "roborev-worker";
      license = pkgs.lib.licenses.mit;
      platforms = ["x86_64-linux" "aarch64-linux"];
    };
  }
