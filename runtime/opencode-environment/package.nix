package:
package.overrideAttrs (old: {
  patches = (old.patches or []) ++ [./legacy.patch];
  postFixup =
    (old.postFixup or "")
    + ''
      wrapProgram "$out/bin/opencode" --set CANIX_TOOLBELT_OPENCODE_REQUIRE_LEGACY 1
    '';
  passthru =
    (old.passthru or {})
    // {
      harborLlmEnvironmentVersion = 1;
      # Transitional capability for the previous Harbor module.
      harborCanixLlmEnvironmentVersion = 1;
    };
})
