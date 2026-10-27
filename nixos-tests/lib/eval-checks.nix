{pkgs}: {
  mkEvalCheck = import ../../lib/mkEvalCheck.nix {inherit pkgs;};
}
