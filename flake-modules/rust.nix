{inputs, ...}: {
  perSystem = {
    pkgs,
    system,
    ...
  }: let
    rustPkgs = pkgs.extend (import inputs.harbor-rs.inputs.rust-overlay);
    rustVersion = (builtins.fromTOML (builtins.readFile ../Cargo.toml)).package.rust-version;
    channel =
      if builtins.length (pkgs.lib.splitString "." rustVersion) == 2
      then "${rustVersion}.0"
      else rustVersion;
    toolchain = inputs.harbor-rs.lib.mkToolchain {
      pkgs = rustPkgs;
      toolchainFile = builtins.toFile "canix-toolbelt-msrv.toml" ''
        [toolchain]
        channel = "${channel}"
        profile = "minimal"
      '';
      withRustAnalyzer = false;
      crossTargets = [];
    };
  in {
    devShells.msrv =
      (inputs.harbor-rs.lib.mkDevShells {
        pkgs = rustPkgs;
        inherit (toolchain) craneLib;
        cross = inputs.harbor-rs.lib.mkCross {
          pkgs = rustPkgs;
          inherit system;
          enableOsxcross = false;
        };
        packages = [rustPkgs.perl];
        opencodeLsp.enable = false;
        extraEnv = {
          RUSTFLAGS = "";
          CARGO_ENCODED_RUSTFLAGS = "";
        };
      }).default;
  };
}
