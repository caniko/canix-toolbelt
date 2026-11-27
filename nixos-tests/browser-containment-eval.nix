{pkgs}: let
  inherit (pkgs) lib;
  package = pkgs.writeShellScriptBin "browser-gpu-guard" "exit 0";
  evaluate = settings:
    (import "${pkgs.path}/nixos/lib/eval-config.nix" {
      inherit pkgs;
      system = null;
      modules = [
        ../modules/nixos/desktop/browser-containment.nix
        {canix-toolbelt.desktop.browserContainment = settings;}
      ];
    }).config;
  disabled = evaluate {};
  enabled = evaluate {
    enable = true;
    inherit package;
    memoryHigh = "8G";
  };
  missing = evaluate {enable = true;};
  slice = enabled.systemd.user.slices.app-browser.sliceConfig;
  service = enabled.systemd.user.services.canix-browser-gpu-guard;
in
  assert !disabled.canix-toolbelt.desktop.browserContainment.enable;
  assert !(disabled.systemd.user.slices ? app-browser);
  assert !(disabled.systemd.user.services ? canix-browser-gpu-guard);
  assert enabled.systemd.oomd.enable;
  assert slice.MemoryAccounting && slice.CPUAccounting;
  assert slice.MemoryHigh == "8G" && slice.MemoryMax == "16G" && slice.MemorySwapMax == "2G";
  assert slice.ManagedOOMMemoryPressure == "kill" && slice.ManagedOOMSwap == "kill";
  assert service.wantedBy == ["graphical-session.target"] && service.partOf == service.wantedBy;
  assert service.serviceConfig.ExecStart == "${package}/bin/browser-gpu-guard";
  assert service.serviceConfig.Restart == "on-failure" && service.serviceConfig.RestartSec == 5;
  assert lib.any (entry: !entry.assertion && lib.hasInfix "browserContainment.package" entry.message) missing.assertions;
    pkgs.runCommand "browser-containment-eval" {} ''touch "$out"''
