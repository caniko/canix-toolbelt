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
pkgs: let
  acpAdapter = pkgs.callPackage ./acp-adapter.nix {};
in {
  amp = [
    pkgs.amp-cli
    (acpAdapter {
      pname = "amp-acp";
      version = "0.7.0";
      srcHash = "sha256-Jwfrt5gESfYYvAuVkT1n5asU2kIZSd5xn9xG3f3hgi4=";
      npmDepsHash = "sha256-lU6T1gqU0ifm3xQx1P4ihOPAyRv8f7ju+6gi4wpK72A=";
      description = "ACP adapter that bridges Amp Code to Agent Client Protocol";
      homepage = "https://github.com/tao12345666333/amp-acp";
      owner = "tao12345666333";
      license = pkgs.lib.licenses.asl20;
    })
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
    (acpAdapter {
      pname = "pi-acp";
      version = "0.0.25";
      srcHash = "sha256-MdEXjHvn8eCy2mPstgTwXUZh99whr8hCA4CTFis1h3g=";
      npmDepsHash = "sha256-GuHvjqSD4M87cGBtFFSF37FWF79+6pLlai0A99Ii/hM=";
      description = "ACP adapter for the Pi coding agent";
      homepage = "https://github.com/svkozak/pi-acp";
      owner = "svkozak";
      license = pkgs.licenses.mit;
    })
  ];
}
