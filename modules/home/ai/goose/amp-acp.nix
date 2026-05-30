{
  lib,
  buildNpmPackage,
  fetchFromGitHub,
  makeBinaryWrapper,
  ripgrep,
}:
buildNpmPackage (finalAttrs: {
  pname = "amp-acp";
  version = "0.7.0";

  src = fetchFromGitHub {
    owner = "tao12345666333";
    repo = "amp-acp";
    tag = "v${finalAttrs.version}";
    hash = "sha256-Jwfrt5gESfYYvAuVkT1n5asU2kIZSd5xn9xG3f3hgi4=";
  };

  npmDepsHash = "sha256-lU6T1gqU0ifm3xQx1P4ihOPAyRv8f7ju+6gi4wpK72A=";

  nativeBuildInputs = [makeBinaryWrapper];

  dontNpmTest = true;

  postFixup = ''
    wrapProgram $out/bin/amp-acp \
      --prefix PATH : ${lib.makeBinPath [ripgrep]}
  '';

  meta = {
    description = "ACP adapter that bridges Amp Code to Agent Client Protocol";
    homepage = "https://github.com/tao12345666333/amp-acp";
    changelog = "https://github.com/tao12345666333/amp-acp/releases/tag/v${finalAttrs.version}";
    license = lib.licenses.asl20;
    mainProgram = "amp-acp";
    platforms = lib.platforms.unix;
  };
})
