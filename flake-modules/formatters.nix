{inputs, ...}: {
  imports = [inputs.treefmt-nix.flakeModule];

  perSystem.treefmt = {
    flakeCheck = true;

    programs = {
      deadnix.enable = true;
      gofmt.enable = true;
      just.enable = true;
      mypy.enable = true;
      ruff-check.enable = true;
      alejandra.enable = true;
      rustfmt.enable = true;
      statix.enable = true;
      terraform.enable = true;
      typos.enable = false;
    };

    projectRootFile = "flake.nix";
  };
}
