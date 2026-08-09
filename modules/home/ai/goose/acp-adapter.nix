{
  lib,
  buildNpmPackage,
  fetchFromGitHub,
  makeBinaryWrapper,
  ripgrep,
}: {
  pname,
  version,
  srcHash,
  npmDepsHash,
  description,
  homepage,
  owner,
  license,
}: let
  attrs = {
    inherit pname version;

    src = fetchFromGitHub {
      inherit owner;
      repo = pname;
      tag = "v${version}";
      hash = srcHash;
    };

    inherit npmDepsHash;

    nativeBuildInputs = [makeBinaryWrapper];

    dontNpmTest = true;

    postFixup = ''
      wrapProgram $out/bin/${pname} \
        --prefix PATH : ${lib.makeBinPath [ripgrep]}
    '';

    meta = {
      inherit description homepage license;
      changelog = "https://github.com/${owner}/${pname}/releases/tag/v${version}";
      mainProgram = pname;
      platforms = lib.platforms.unix;
    };
  };
in
  buildNpmPackage attrs
