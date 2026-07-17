{lib}: {
  mkAttachExecWrapper = {
    pkgs,
    name ? "dev-agent-exec",
    unit,
    targetName,
    pathMarkers ? [],
    targetPath ? null,
  }: let
    configFile = pkgs.writeText "${name}-config.json" (builtins.toJSON {
      inherit unit targetName pathMarkers targetPath;
      busctl = "${pkgs.systemd}/bin/busctl";
    });
  in
    pkgs.writers.writePython3 name {} ''
      import json
      import os
      import subprocess
      import sys


      def fail(message):
          print(f"{sys.argv[0]}: {message}", file=sys.stderr)
          raise SystemExit(125)


      def find_target(config):
          if config.get("targetPath"):
              target = config["targetPath"]
              if os.access(target, os.X_OK):
                  return target
              fail(f"configured target is not executable: {target}")

          markers = config.get("pathMarkers", [])
          for directory in reversed(os.environ.get("PATH", "").split(os.pathsep)):
              if not directory:
                  continue
              target = os.path.join(directory, config["targetName"])
              if not os.access(target, os.X_OK):
                  continue
              if all(
                  os.path.exists(os.path.join(directory, marker))
                  for marker in markers
              ):
                  return target
          fail(
              f"could not find {config['targetName']} with markers "
              f"{markers!r} in PATH"
          )


      def unified_cgroup():
          with open("/proc/self/cgroup", encoding="ascii") as stream:
              for line in stream:
                  hierarchy, separator, path = line.rstrip("\n").partition("::")
                  if separator and hierarchy == "0":
                      return path
          return ""


      config_path = "${configFile}"  # noqa: E501
      with open(config_path, encoding="utf-8") as stream:
          config = json.load(stream)

      target = find_target(config)
      result = subprocess.run(
          [
              config["busctl"],
              "--user",
              "call",
              "org.freedesktop.systemd1",
              "/org/freedesktop/systemd1",
              "org.freedesktop.systemd1.Manager",
              "AttachProcessesToUnit",
              "ssau",
              config["unit"],
              "",
              "1",
              str(os.getpid()),
          ],
          check=False,
          text=True,
          stdout=subprocess.PIPE,
          stderr=subprocess.PIPE,
      )
      if result.returncode != 0:
          detail = result.stderr.strip() or result.stdout.strip() or "unknown error"
          fail(f"could not attach to {config['unit']}: {detail}")
      if config["unit"] not in unified_cgroup():
          fail(
              f"attachment to {config['unit']} was not reflected "
              "in /proc/self/cgroup"
          )

      os.execve(target, [target, *sys.argv[1:]], os.environ.copy())
    '';
}
