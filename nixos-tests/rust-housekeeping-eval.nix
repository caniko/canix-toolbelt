{pkgs}: let
  inherit (pkgs) lib;
  evaluate = settings:
    (import "${pkgs.path}/nixos/lib/eval-config.nix" {
      inherit pkgs;
      system = null;
      modules = [
        ../modules/nixos/services/rust-housekeeping.nix
        {canix-toolbelt.services.rustHousekeeping = settings;}
      ];
    }).config;
  disabled = evaluate {};
  enabled = evaluate {
    enable = true;
    user = "builder";
    home = "/srv/builder";
  };
  service = enabled.systemd.services.canix-rust-housekeeping;
  timer = enabled.systemd.timers.canix-rust-housekeeping;
  contract = enabled.canix-toolbelt.activation.contracts.rustHousekeeping;
in
  assert !disabled.canix-toolbelt.services.rustHousekeeping.enable;
  assert !(disabled.systemd.services ? canix-rust-housekeeping);
  assert !(disabled.systemd.timers ? canix-rust-housekeeping);
  assert disabled.canix-toolbelt.activation.contracts.rustHousekeeping.status == "disabled";
  assert service.serviceConfig.User == "builder";
  assert service.serviceConfig.WorkingDirectory == "/srv/builder";
  assert service.environment.HOME == "/srv/builder";
  assert service.environment.CARGO_HOME == "/srv/builder/.cargo";
  assert service.serviceConfig.ExecStart == "${lib.getExe pkgs.cargo-cache} --autoclean";
  assert service.startAt == ["weekly"];
  assert timer.timerConfig.Persistent;
  assert timer.timerConfig.RandomizedDelaySec == "30m";
  assert contract.status == "deferred";
  assert contract.requirements.cargo-home.phase == "runtime";
  assert (builtins.head contract.artifacts).path == "/srv/builder/.cargo";
  assert contract.units == ["canix-rust-housekeeping.service" "canix-rust-housekeeping.timer"];
    pkgs.runCommand "rust-housekeeping-eval" {} ''touch "$out"''
