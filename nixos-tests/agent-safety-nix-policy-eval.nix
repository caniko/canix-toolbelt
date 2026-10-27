{pkgs}: let
  inherit (pkgs) lib;
  toolbelt = import ../lib {inherit lib;};
  inherit (import ./lib/eval-checks.nix {inherit pkgs;}) mkEvalCheck;

  rulesFor = args: toolbelt.agentSafety.mergeRules (toolbelt.agentSafetyNixPolicy args);
  default = rulesFor {};
  facade = rulesFor {storeAddAsk = false;};

  # OpenCode semantics: the last matching rule in attribute-name order wins.
  globRegex = pattern: builtins.replaceStrings ["\\*" "\\?"] [".*" "."] (lib.escapeRegex pattern);
  decide = rules: command: rules.${lib.last (lib.filter (pattern: builtins.match (globRegex pattern) command != null) (builtins.attrNames rules))};

  cases = [
    [default "nix eval --no-allow-import-from-derivation .#foo" "allow"]
    [default "FOO=1 nix eval --no-allow-import-from-derivation .#foo" "allow"]
    [default "nix eval .#foo" "deny"]
    [default "nix eval --no-allow-import-from-derivation --allow-import-from-derivation .#foo" "deny"]
    [default "nix build --dry-run --no-allow-import-from-derivation .#foo --no-dry-run" "deny"]
    [default "FOO=1 nix build .#foo" "deny"]
    [default "./result/bin/nix build .#foo" "deny"]
    [default "/nix/store/abc-nix/bin/nix store ls /nix/store/abc-foo" "allow"]
    [default "nix path-info /nix/store/abc-foo" "allow"]
    [default "nix path-info /nix/store/abc-foo --expr x" "deny"]
    [default "nix-store --query --references /nix/store/abc-foo" "allow"]
    [default "nix-store --query --realise /nix/store/abc-foo" "deny"]
    [default "nix-store --query -f /nix/store/abc-foo.drv" "deny"]
    [default "nix-store -q -Rf /nix/store/abc-foo.drv" "deny"]
    [default "nix-store --query --references /nix/store/abc-foo.drv -u" "deny"]
    [default "nix-store --query --references /nix/store/abc-fuse-3.16 /nix/store/abc-gnu-efi-4" "allow"]
    [default "nix eval --no-allow-import-from-derivation --write-to ./out .#foo" "deny"]
    [default "nix flake archive ." "allow"]
    [default "nix flake archive --to ssh://builder ." "ask"]
    [default "nix shell nixpkgs#hello" "ask"]
    [default "nix store add ./file" "ask"]
    [facade "nix store add ./file" "deny"]
  ];
in
  mkEvalCheck {
    name = "agent-safety-nix-policy-eval";
    resultMessage = "generic Nix command policy resolves IFD-safe allows and suffix denies";
    assertions =
      map (case: let
        rules = builtins.elemAt case 0;
        command = builtins.elemAt case 1;
        expected = builtins.elemAt case 2;
        actual = decide rules command;
      in {
        name = command;
        assertion = actual == expected;
        message = "expected ${expected}, got ${actual}";
      })
      cases;
  }
