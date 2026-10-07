{lib}: rec {
  mkPackages = {
    pkgs,
    zellij ? pkgs.zellij,
    gnupg ? pkgs.gnupg,
    rage ? pkgs.rage,
    tty ? pkgs.pinentry-tty,
    qt ? pkgs.pinentry-qt,
  }: let
    router = pkgs.callPackage ../nix/pinentry.nix {
      inherit zellij gnupg rage;
      pinentry-tty = tty;
      pinentry-qt = qt;
    };
    requestGpg = pkgs.symlinkJoin {
      name = "gnupg-pinentry-context-${gnupg.version}";
      inherit (gnupg) version;
      meta = gnupg.meta // {outputsToInstall = ["out"];};
      paths = map (output: gnupg.${output}) (gnupg.meta.outputsToInstall or ["out"]);
      postBuild = ''
        rm "$out/bin/gpg"
        ln -s ${lib.getExe router} "$out/bin/gpg"
      '';
    };
    requestRage = pkgs.symlinkJoin {
      name = "rage-pinentry-context-${rage.version}";
      inherit (rage) version;
      paths = [rage];
      nativeBuildInputs = [pkgs.makeWrapper pkgs.python3 pkgs.bash pkgs.xorg.xorgserver pkgs.xdotool];
      postBuild = ''
        wrapProgram "$out/bin/rage" \
          --set PINENTRY_PROGRAM ${lib.getExe router} \
          --prefix PATH : ${lib.makeBinPath [router]}
        # symlinkJoin uses buildCommand, so test the installed wrapper here.
        ${lib.getExe pkgs.python3} ${../runtime/pinentry}/test.py \
          ${router}/bin/canix-toolbelt-pinentry-agent ${lib.getExe zellij} \
          ${gnupg}/bin/gpg-connect-agent ${gnupg}/bin/gpgconf ${lib.getExe pkgs.bash} \
          ${gnupg}/bin/gpg ${lib.getExe router} "$out/bin/rage" \
          ${pkgs.xorg.xorgserver}/bin/Xvfb ${lib.getExe pkgs.xdotool} --wrapped-rage \
          PinentryIntegration.test_rage_plugin_confirm_and_pin_use_direct_context_with_piped_stdio \
          PinentryIntegration.test_rage_plugin_cancel_does_not_open_a_second_prompt
      '';
      meta = rage.meta // {outputsToInstall = ["out"];};
    };
  in {
    inherit router;
    gpg = requestGpg;
    rage = requestRage;
  };

  mkRage = args: (mkPackages args).rage;
}
