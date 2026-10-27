{pkgs}:
pkgs.writeShellApplication {
  name = "build-memory-probe";
  runtimeInputs = [pkgs.coreutils pkgs.gawk pkgs.jq pkgs.systemd];
  text = builtins.readFile ./build-memory-probe.sh;
}
