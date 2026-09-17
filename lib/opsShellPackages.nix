# The standard tooling bundle for an ops devShell on a NixOS fleet:
#
#   - attic-client (binary cache pushes)
#   - rage, age-plugin-fido2-hmac (age secrets)
#   - hardware introspection: pciutils, lshw, nvme-cli, gptfdisk,
#     efibootmgr, libfido2, mesa-demos
#   - openssl, ripgrep
#
# Returned as a list of packages so it composes with arbitrary shell builders
# (`pkgs.mkShell`, harbor-rs's `mkDevShell`, etc).
#
# Used internally by `flakeModules.ops-shell`; expose separately so consumers
# can plug it into their own builder.
pkgs:
with pkgs; [
  attic-client
  rage
  age-plugin-fido2-hmac
  ripgrep
  pciutils
  lshw
  nvme-cli
  gptfdisk
  efibootmgr
  libfido2
  openssl
  mesa-demos
]
