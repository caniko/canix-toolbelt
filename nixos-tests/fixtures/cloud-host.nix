# This is an offline fixture, not an enrolled Fleetix host.
{pkgs, ...}: {
  networking.hostName = "cloud-fixture";
  canix-toolbelt.cloudHost = {
    enable = true;
    stateVersion = "25.11";
    boot.mode = "uefi";
    disk = "/dev/disk/by-id/fixture-root";
    network.interfaceName = "ens3";
    access.authorizedKeys = [(import "${pkgs.path}/nixos/tests/ssh-keys.nix" pkgs).snakeOilEd25519PublicKey];
  };
}
