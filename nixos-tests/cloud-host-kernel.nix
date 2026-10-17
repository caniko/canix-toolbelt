# Linux 6.18.52 predates the upstream CPA/text-poke locking fixes. Keep the
# backport test-local and select it for both Disko's installer and installed
# guest. Retire this when the locked kernel has the
# fixes; do not carry these version-specific patches onto a different kernel.
{pkgs}: let
  backports = [
    {
      name = "x86-cpa-collapse-write-lock";
      patch = ./patches/linux-6.18.52-cpa-write-lock.patch;
    }
    {
      name = "x86-cpa-attribute-read-lock";
      patch = ./patches/linux-6.18.52-cpa-read-lock.patch;
    }
    {
      name = "x86-text-poke-cpa-read-lock";
      patch = pkgs.fetchurl {
        url = "https://github.com/torvalds/linux/commit/d14648849f1b1616aa91475026991c8bf78519d4.patch";
        hash = "sha256-MBEVC9PPGkNco1ENKPQwaocrx4FZnAMOpEbydiTsprw=";
      };
    }
  ];
in
  if pkgs.linuxPackages.kernel.version != "6.18.52"
  then pkgs
  else
    pkgs.extend (_: prev: {
      linuxPackages = prev.linuxPackages.extend (_: linuxPrev: {
        kernel = linuxPrev.kernel.override (args: {
          kernelPatches = (args.kernelPatches or []) ++ backports;
        });
      });
    })
