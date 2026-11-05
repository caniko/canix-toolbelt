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
  version = "0.2.40";

  src = fetchFromGitHub {
    owner = "BlizzNux";
    repo = "launcher";
    tag = "v${version}";
    hash = "sha256-ryFx1rj1z5vIHTad9oR9S96Q1EbZIrKl00m1lLLRqJk=";
  };

  # The UI is static (ui/), so the Tauri binary is the only build product; its
  # resource lookup falls back to `$out/bin/blizznux-run` next to the binary.
  cargoRoot = "src-tauri";
  buildAndTestSubdir = "src-tauri";
  cargoHash = "sha256-xAquLmQCEpk9DUZ2vOCunVG1LgYknvyiELI9jgbmBqw=";

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
