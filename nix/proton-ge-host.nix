# nixpkgs' GE-Proton, run by umu-launcher on the host instead of inside the
# Steam Linux Runtime.
#
# umu-launcher reads the runtime a tool needs from its toolmanifest.vdf and
# downloads that Steam Linux Runtime on first use. Without `require_tool_appid`
# it runs Proton directly (umu's "host" runtime), and nixpkgs' umu-launcher is
# itself an FHS environment that holds Proton's libraries, so nothing is
# downloaded at runtime. The FHS has no 32-bit graphics drivers: run 32-bit
# programs with PROTON_USE_WOW64=1.
{
  proton-ge-bin,
  runCommandLocal,
}: let
  proton = proton-ge-bin.steamcompattool;
in
  runCommandLocal "proton-ge-host-${proton-ge-bin.version}" {
    meta = {
      description = "GE-Proton (${proton-ge-bin.version}) for umu-launcher's host runtime";
      inherit (proton-ge-bin.meta) homepage license platforms;
      mainProgram = "proton";
    };
  } ''
    mkdir $out
    for f in ${proton}/*; do
      [[ ''${f##*/} == toolmanifest.vdf ]] || ln -s "$f" $out/
    done
    grep -v require_tool_appid ${proton}/toolmanifest.vdf > $out/toolmanifest.vdf
  ''
