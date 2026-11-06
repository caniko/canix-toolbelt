# A Proton compatibility tool whose Wine is a 64-bit-only WoW64 build.
#
# A new-WoW64 Wine (`--enable-archs=x86_64,i386`) runs 32-bit Windows binaries
# inside the 64-bit Unix process: it installs `i386-windows` PE modules but no
# `i386-unix` libraries, so neither the host's 32-bit loader nor 32-bit
# graphics drivers are needed.
#
# Proton is kept intact — launcher script, DXVK, VKD3D-Proton, nvapi, the
# default prefix — and only Wine is swapped for the WoW64 build, so the tree
# stays a Proton that umu-launcher and the BlizzNux wrapper can drive.
{
  pkgs,
  proton ? pkgs.proton-ge-bin,
  # Not named `wine`: callPackage would fill a `wine` argument from pkgs.wine,
  # which is the 32-bit build.
  winePackage ? pkgs.wineWow64Packages.stable,
}:
pkgs.runCommandLocal "proton-wow64-${winePackage.version}" {
  meta = {
    description = "Proton (${proton.version}) with a 64-bit-only WoW64 Wine (${winePackage.version})";
    inherit (proton.meta) homepage license platforms;
    mainProgram = "proton";
  };
} ''
  mkdir -p $out
  # proton-ge-bin's steamcompattool output is a tree of symlinks into the
  # unpacked release; copy through them so the result can be modified.
  cp -rL ${proton.steamcompattool}/. $out/
  chmod -R u+w $out

  # Wine's own binaries and PE/unix modules, version-matched to `winePackage`.
  cp -a ${winePackage}/bin/. $out/files/bin/
  for d in i386-windows x86_64-windows; do
    # Overlay, not replace: Proton ships extra PE modules this Wine build does
    # not produce (vrclient, lsteamclient, amd_ags, nvapi, cuda/opencl shims)
    # and its launcher copies some of them into the prefix.
    cp -a ${winePackage}/lib/wine/$d/. "$out/files/lib/wine/$d/"
  done
  rm -rf $out/files/lib/wine/x86_64-unix
  cp -a ${winePackage}/lib/wine/x86_64-unix $out/files/lib/wine/x86_64-unix
  rm -rf $out/files/share/wine
  cp -a ${winePackage}/share/wine $out/files/share/wine

  # A new-WoW64 build has no 32-bit Unix side; Proton ships one.
  rm -rf $out/files/lib/wine/i386-unix

  # `version` is left as Proton's: the launcher compares it against the prefix
  # on upgrade. This file records what actually runs.
  echo ${winePackage.version} > $out/files/wine-wow64-version
''
