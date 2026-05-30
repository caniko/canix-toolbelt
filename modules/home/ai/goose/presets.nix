# ACP/agent provider preset package sets for `programs.goose`.
#
# `acpPackages pkgs` returns an attrset keyed by goose ACP/agent provider name
# (without the `-acp`/`-agent` suffix) holding the package(s) that provide the
# adapter command on PATH. Hosts opt in per provider, e.g.:
#
#   programs.goose.acp.providers.claude.enable = true;
#   programs.goose.acp.providers.claude.packages =
#     (canix-toolbelt.lib.goose.acpPackages pkgs).claude;
#
# Each set is only evaluated when referenced, so providers whose packages are
# absent on a given host cost nothing unless actually wired up.
pkgs: {
  amp = [
    pkgs.amp-cli
    (pkgs.callPackage ./amp-acp.nix {})
  ];
  claude = [
    pkgs.claude-code
    pkgs.claude-agent-acp
  ];
  copilot = [
    pkgs.github-copilot-cli
  ];
  pi = [
    pkgs.pi-coding-agent
    (pkgs.callPackage ./pi-acp.nix {})
  ];
}
