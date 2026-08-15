# systemd — helpers for common systemd unit patterns.
#
#   mkResumableOperator — ordered, resumable systemd stage controller
{lib}: {
  mkResumableOperator = import ./resumable-operator.nix {inherit lib;};
}
