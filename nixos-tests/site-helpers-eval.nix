{pkgs, ...}: let
  toolbeltLib = import ../lib {inherit (pkgs) lib;};

  zolaFixture = pkgs.runCommand "site-helper-zola-fixture" {} ''
    mkdir -p "$out/content" "$out/templates"
    cat > "$out/config.toml" <<'EOF'
    base_url = "https://example.test"
    title = "Fixture"
    compile_sass = false
    build_search_index = false
    EOF
    cat > "$out/content/_index.md" <<'EOF'
    +++
    title = "Home"
    +++
    Hello from Zola.
    EOF
    cat > "$out/templates/index.html" <<'EOF'
    <!doctype html>
    <html><head><title>{{ section.title }}</title></head><body>{{ section.content | safe }}</body></html>
    EOF
  '';

  mdBookFixture = pkgs.runCommand "site-helper-mdbook-fixture" {} ''
    mkdir -p "$out/src"
    cat > "$out/book.toml" <<'EOF'
    [book]
    title = "Fixture Book"
    EOF
    cat > "$out/src/SUMMARY.md" <<'EOF'
    # Summary

    - [Home](index.md)
    EOF
    cat > "$out/src/index.md" <<'EOF'
    # Home

    Hello from mdBook.
    EOF
  '';

  zolaSite = toolbeltLib.mkZolaSite {
    inherit pkgs;
    src = zolaFixture;
    pname = "site-helper-zola-site";
    dataFiles."data/generated.toml" = pkgs.writeText "generated.toml" ''
      value = 1
    '';
  };

  mdBookDocs = toolbeltLib.mkMdBookDocs {
    inherit pkgs;
    src = mdBookFixture;
    pname = "site-helper-mdbook-docs";
  };

  combinedWithDomains = toolbeltLib.mkCombinedSite {
    inherit pkgs;
    website = zolaSite;
    docs = mdBookDocs;
    pname = "site-helper-combined-with-domains";
    domains = ["test.example"];
  };

  combinedWithoutDomains = toolbeltLib.mkCombinedSite {
    inherit pkgs;
    website = zolaSite;
    docs = mdBookDocs;
    pname = "site-helper-combined-without-domains";
    domains = null;
  };
in
  pkgs.runCommand "site-helpers-eval" {} ''
    test -f ${zolaSite}/index.html
    test -f ${mdBookDocs}/index.html
    test -f ${combinedWithDomains}/index.html
    test -f ${combinedWithDomains}/docs/index.html
    test -f ${combinedWithDomains}/.domains
    head -n 1 ${combinedWithDomains}/.domains | grep -qx test.example
    test ! -e ${combinedWithoutDomains}/.domains
    mkdir -p "$out"
    printf '%s\n' "site helpers evaluated and built fixtures" > "$out/result"
  ''
