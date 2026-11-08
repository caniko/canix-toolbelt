{
  lib,
  pkgs,
  rustPlatform,
  fetchFromGitHub,
  pkg-config,
  wrapGAppsHook4,
  cairo,
  gdk-pixbuf,
  glib,
  gtk3,
  libsoup_3,
  librsvg,
  pango,
  vulkan-loader,
  webkitgtk_4_1,
  # BlizzNux pins and downloads its own Proton at runtime. Use a Nix-provided
  # compatibility tool instead, so the version is reproducible and nothing is
  # fetched into the user's data directory.
  proton ? pkgs.proton-ge-bin,
}:
rustPlatform.buildRustPackage rec {
  pname = "blizznux";
  version = "0.2.42-unstable-2026-11-08";

  # BlizzNux/launcher#47 (Labs: Wine WoW64 mode): its readiness check stops
  # asking for 32-bit Vulkan drivers, which this WoW64-only Wine never loads.
  # Back to a release tag once it ships in one.
  src = fetchFromGitHub {
    owner = "BlizzNux";
    repo = "launcher";
    rev = "53eccbd6e734aadf6fe5c14aed71e2828dc67db0";
    hash = "sha256-2NUG/j5kV4udqcr+GPNW+KsRFUXf7dUP+2YtNWjeAcU=";
  };

  # The UI is static (ui/), so the Tauri binary is the only build product; its
  # resource lookup falls back to `$out/bin/blizznux-run` next to the binary.
  cargoRoot = "src-tauri";
  buildAndTestSubdir = "src-tauri";
  cargoHash = "sha256-Fr3wiw1fGw9JTSyjWJYU7skEnmbcuQUJ2yaMjkuLqDI=";

  # blizznux-run.patch moves the default prefix to Proton's <data>/pfx. The
  # launcher must default there too: settings saved before the first install
  # (the Labs toggle, say) write an empty PREFIX, and it would then look for
  # Battle.net in a <data>/prefix that never gets it.
  postPatch = ''
    substituteInPlace src-tauri/src/lib.rs \
      --replace-fail '.join("blizznux").join("prefix")' '.join("blizznux").join("pfx")'
  '';

  nativeBuildInputs = [pkg-config wrapGAppsHook4];
  buildInputs = [cairo gdk-pixbuf glib gtk3 libsoup_3 librsvg pango webkitgtk_4_1];

  # `blizznux-run` installs and launches Battle.net through umu-launcher, and
  # would fetch its own Proton into the user's data directory. The patch runs
  # Proton directly (umu's Steam Runtime container cannot see the /nix/store
  # this Wine lives in) and points it at a symlink under $out, so neither Wine
  # nor umu is downloaded at runtime.
  #
  # `proton` is either a compatibility-tool package (proton-wow64) or one with a
  # separate `steamcompattool` output (proton-ge-bin).
  postInstall = let
    protonTool =
      proton.steamcompattool or proton;
  in ''
    patch -p1 < ${./blizznux-run.patch}
    install -d $out/share/blizznux/proton
    ln -s ${protonTool} $out/share/blizznux/proton/${proton.name}
    substituteInPlace bin/blizznux-run \
      --replace-fail 'PROTON_DIR=$DATA_DIR/proton' "PROTON_DIR=$out/share/blizznux/proton" \
      --replace-fail 'PROTON_NAME=proton-$PROTON_VERSION-x86_64' "PROTON_NAME=${proton.name}" \
      --replace-fail '@vulkanLib@' "${vulkan-loader}/lib"

    install -Dm755 bin/blizznux-run $out/bin/blizznux-run
    install -Dm644 share/applications/com.blizznux.launcher.desktop $out/share/applications/com.blizznux.launcher.desktop
    install -d $out/share/icons
    cp -a share/icons/hicolor $out/share/icons/
  '';

  meta = {
    description = "Battle.net games on Linux through Proton, no Steam needed";
    homepage = "https://github.com/BlizzNux/launcher";
    license = lib.licenses.gpl3Plus;
    mainProgram = "blizznux";
    platforms = ["x86_64-linux"];
    sourceProvenance = [lib.sourceTypes.fromSource];
  };
}
