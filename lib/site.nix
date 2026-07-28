{lib}: let
  inherit (builtins) attrNames baseNameOf dirOf isAttrs;

  validateRelPath = relPath:
    if lib.hasPrefix "/" relPath || lib.hasInfix "../" relPath || lib.hasInfix "/.." relPath
    then throw "site helper paths must be relative paths inside the site tree: ${relPath}"
    else relPath;

  copyDataFile = relPath: source: let
    checkedRelPath = validateRelPath relPath;
    target = "site/${checkedRelPath}";
    targetDir = dirOf target;
  in ''
    mkdir -p ${lib.escapeShellArg targetDir}
    cp -rL ${lib.escapeShellArg (toString source)} ${lib.escapeShellArg target}
  '';

  copyDataFiles = dataFiles:
    lib.concatStringsSep "\n" (map (relPath: copyDataFile relPath dataFiles.${relPath}) (attrNames dataFiles));

  themeCommands = theme:
    if theme == null
    then ""
    else let
      themeName =
        if isAttrs theme && theme ? name && theme ? src
        then validateRelPath theme.name
        else throw "mkZolaSite theme must be null or an attrset with { name, src }";
    in ''
      mkdir -p ${lib.escapeShellArg "site/themes/${themeName}"}
      cp -rL ${lib.escapeShellArg "${theme.src}/."} ${lib.escapeShellArg "site/themes/${themeName}/"}
    '';

  domainsCommands = domains:
    if domains == null
    then ""
    else if domains == []
    then throw "mkCombinedSite domains must be null or a non-empty list; Codeberg Pages treats the first line as canonical"
    else ''
      printf '%s\n' ${lib.concatMapStringsSep " " lib.escapeShellArg domains} > "$out/.domains"
    '';
in {
  mkZolaSite = {
    pkgs,
    src,
    pname ? baseNameOf (toString src),
    version ? "0.1.0",
    dataFiles ? {},
    theme ? null,
  }:
    pkgs.stdenv.mkDerivation {
      inherit pname version src;

      nativeBuildInputs = [pkgs.zola];

      phases = ["buildPhase" "installPhase"];

      buildPhase = ''
        cp -r --no-preserve=mode "$src" site
        chmod -R u+w site
        ${copyDataFiles dataFiles}
        ${themeCommands theme}
        cd site
        zola build
      '';

      installPhase = ''
        mkdir -p "$out"
        cp -r public/. "$out/"
      '';
    };

  mkMdBookDocs = {
    pkgs,
    src,
    pname ? baseNameOf (toString src),
    version ? "0.1.0",
  }:
    pkgs.stdenv.mkDerivation {
      inherit pname version src;

      nativeBuildInputs = [pkgs.mdbook];

      phases = ["buildPhase" "installPhase"];

      buildPhase = ''
        cp -r --no-preserve=mode "$src" docs
        chmod -R u+w docs
        mdbook build docs
      '';

      installPhase = ''
        mkdir -p "$out"
        cp -r docs/book/. "$out/"
      '';
    };

  # Codeberg Pages reads the first line of .domains as the canonical domain.
  # Pass domains = null to omit .domains entirely; when passing a list, order
  # matters.
  mkCombinedSite = {
    pkgs,
    website,
    docs,
    pname ? "combined-site",
    docsPath ? "docs",
    domains ? null,
  }: let
    checkedDocsPath = validateRelPath docsPath;
  in
    pkgs.runCommand pname {} ''
      mkdir -p "$out" "$out/${checkedDocsPath}"
      cp -rL ${website}/. "$out/"
      cp -rL ${docs}/. "$out/${checkedDocsPath}/"
      ${domainsCommands domains}
    '';

}
