{pkgs, ...}: {
  hardware.graphics.extraPackages = with pkgs; [
    intel-compute-runtime
    level-zero
  ];

  environment.systemPackages = with pkgs; [
    clinfo
  ];
}
