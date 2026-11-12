{
  lib,
  rustPlatform,
  fetchFromGitHub,
  pkg-config,
  wrapGAppsHook4,
  cairo,
  curl,
  gdk-pixbuf,
  glib,
  gtk3,
  libsoup_3,
  librsvg,
  pango,
  python3,
  vulkan-loader,
  webkitgtk_4_1,
}:
rustPlatform.buildRustPackage {
  pname = "blizznux";
  version = "0.2.44-unstable-2026-11-12";

  # BlizzNux/launcher main (#62: finds Vulkan drivers on NixOS) plus the
  # packager-locked Labs settings (BLIZZNUX_LAB_*), pinned at the fork PR head
  # until it merges upstream. Back to a release tag once both ship in one.
  src = fetchFromGitHub {
    owner = "caniko";
    repo = "launcher";
    rev = "28722559e90812559a016199a91b598ddb28f114";
    hash = "sha256-2+CHp5O4bAEqXWccohLlFv9tq9e6LHBLAXiZz0iXpzg=";
  };

  # The UI is static (ui/), so the Tauri binary is the only build product; its
  # resource lookup falls back to `$out/bin/blizznux-run` next to the binary.
  cargoRoot = "src-tauri";
  buildAndTestSubdir = "src-tauri";
  cargoHash = "sha256-UXwhzI/E+PLfnbz/EIwivlJ6ECO+7uX7+9rTsPB5C+g=";

  nativeBuildInputs = [pkg-config wrapGAppsHook4];
  buildInputs = [cairo gdk-pixbuf glib gtk3 libsoup_3 librsvg pango webkitgtk_4_1];

  # Proton and the Labs modes come from the NixOS module (programs.blizznux.proton
  # and .labs): a Nix-built Proton must run directly, since umu's Steam Runtime
  # container cannot see the /nix/store it lives in.
  postInstall = ''
    install -Dm755 bin/blizznux-run $out/bin/blizznux-run
    install -Dm644 share/applications/com.blizznux.launcher.desktop $out/share/applications/com.blizznux.launcher.desktop
    install -d $out/share/icons
    cp -a share/icons/hicolor $out/share/icons/
  '';

  # Wraps the app and blizznux-run: curl fetches Blizzard's installer, python3
  # runs the `proton` script, and Proton probes libvulkan through ctypes before
  # Wine starts.
  preFixup = ''
    gappsWrapperArgs+=(
      --prefix PATH : ${lib.makeBinPath [curl python3]}
      --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath [vulkan-loader]}
    )
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
