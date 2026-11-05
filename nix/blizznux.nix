{
  lib,
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
  webkitgtk_4_1,
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

  # `blizznux-run` installs and launches Battle.net through umu-launcher and
  # Proton, which it downloads on first use into the user's data directory.
  postInstall = ''
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
