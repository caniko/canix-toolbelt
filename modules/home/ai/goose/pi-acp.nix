{
  lib,
  buildNpmPackage,
  fetchFromGitHub,
  makeBinaryWrapper,
  ripgrep,
}:
buildNpmPackage (finalAttrs: {
  pname = "pi-acp";
  version = "0.0.25";

  src = fetchFromGitHub {
    owner = "svkozak";
    repo = "pi-acp";
    tag = "v${finalAttrs.version}";
    hash = "sha256-MdEXjHvn8eCy2mPstgTwXUZh99whr8hCA4CTFis1h3g=";
  };

  npmDepsHash = "sha256-GuHvjqSD4M87cGBtFFSF37FWF79+6pLlai0A99Ii/hM=";

  nativeBuildInputs = [makeBinaryWrapper];

  dontNpmTest = true;

  postFixup = ''
    wrapProgram $out/bin/pi-acp \
      --prefix PATH : ${lib.makeBinPath [ripgrep]}
  '';

  meta = {
    description = "ACP adapter for the Pi coding agent";
    homepage = "https://github.com/svkozak/pi-acp";
    changelog = "https://github.com/svkozak/pi-acp/releases/tag/v${finalAttrs.version}";
    license = lib.licenses.mit;
    mainProgram = "pi-acp";
    platforms = lib.platforms.unix;
  };
})
