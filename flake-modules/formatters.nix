{inputs, ...}: {
  imports = [inputs.treefmt-nix.flakeModule];

  perSystem.treefmt = {
    flakeCheck = true;

    settings.excludes = [
      "nix/store/**"
    ];

    programs = {
      deadnix.enable = true;
      gofmt.enable = true;
      just.enable = true;
      ruff-check.enable = true;
      alejandra.enable = true;
      rustfmt.enable = true;
      # Shell scripts are formatted through treefmt only; agents may not
      # run shfmt directly (see the project-side formatter deny policy).
      shfmt.enable = true;
      statix.enable = true;
      terraform.enable = true;
      typos.enable = false;
    };

    projectRootFile = "flake.nix";
  };
}
