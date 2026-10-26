# Generic Bash prompt policy for Nix commands, in `agentSafety.mergeRules`
# allow/ask/deny shape. Consumers append their own host-specific exceptions.
#
# Evaluation-shaped commands are allow-listed only with
# `--no-allow-import-from-derivation` in a fixed position: even
# `--dry-run`/`--no-build` can realise import-from-derivation outputs, and a
# trailing ` *` would otherwise also match the flag without it. The longer
# suffix denies below sort after their allow prefix and reject anything that
# re-enables it later on the command line.
#
# Deny coverage spans bare, environment-prefixed (`FOO=1 nix ...`), and
# result-bin invocations. A `/nix/store/.../bin/nix` deny is deliberately
# absent: its literal prefix collides with every program's generated store
# wrapper in `mergeRules`, so those stay default ask instead of silent allow.
# This remains a prompt policy, not a sandbox. The explicit ask list keeps
# uncommon operations accessible; builds and checks belong to a consumer route.
{lib}: {
  # False drops the `nix store add*` asks so the broad denies cover them, e.g.
  # when a consumer routes store additions through its own reason-gated facade.
  storeAddAsk ? true,
}: let
  evaluationPrefixes = map (command: "${command} --no-allow-import-from-derivation ") [
    "nix eval"
    "nix build --dry-run"
    "nix flake check --no-build"
    "nix flake show"
    "nix derivation show"
    "nix path-info"
    "nix search"
    "nix why-depends"
    "nix log"
    "nix-instantiate --eval"
  ];
  storeQueryPrefixes = ["nix-store --query " "nix-store -q " "nix-store --read-log "];
  # Store-bin suffix denies collide with every program's generated wrapper
  # in mergeRules; guarded queries retain default ask for that invocation form.
  queryWrappers = [
    ""
    "*=* "
    "./result/bin/"
    "*=* ./result/bin/"
  ];
  inspectionWrappers =
    queryWrappers
    ++ [
      "/nix/store/*/bin/"
      "*=* /nix/store/*/bin/"
    ];
  readOnlyCommands = [
    "nix --version *"
    "nix --help *"
    "nix * --help"
    "nix help *"
    "nix help-stores *"
    "nix config show *"
    "nix show-config *"
    "nix hash *"
    "nix nar ls *"
    "nix nar cat *"
    "nix nar dump-path *"
    "nix store ls *"
    "nix store cat *"
    "nix store ping *"
    "nix store info *"
    "nix store diff-closures *"
    "nix store dump-path *"
    "nix store path-from-hash-part *"
    "nix registry list *"
    "nix profile list *"
    "nix profile history *"
  ];
  # These shortcuts accept one literal, opaque store path. Options precede
  # that path; trailing arguments, shell expansions and dynamic derivations
  # need the IFD-disabled evaluation route. A glob is not an argv validator.
  storeMetadataPrefixes = map (command: "${command} /nix/store/") [
    "nix path-info"
    "nix path-info --json"
    "nix path-info --json --json-format 1"
    "nix path-info --json --json-format 2"
    "nix path-info --size"
    "nix path-info --recursive"
    "nix path-info -r"
    "nix path-info --recursive --closure-size"
    "nix path-info -rS"
    "nix derivation show"
    "nix derivation show --recursive"
    "nix derivation show -r"
    "nix log"
  ];
  wrapped = wrappers: patterns: lib.concatMap (wrapper: map (pattern: "${wrapper}${pattern}") patterns) wrappers;
in {
  allow =
    [
      "nix flake metadata *"
      "nix flake archive *"
      # Download/unpack/hash only; neither command realises derivations.
      "nix store prefetch-file *"
      "nix-prefetch-url *"
      "nix-instantiate --parse *"
    ]
    ++ wrapped inspectionWrappers readOnlyCommands
    ++ wrapped queryWrappers (map (prefix: "${prefix}*") (storeMetadataPrefixes ++ storeQueryPrefixes))
    ++ wrapped queryWrappers (map (prefix: "${prefix}*") evaluationPrefixes);

  ask =
    [
      "nix bundle *"
      "nix copy *"
      "nix develop *"
      "nix edit *"
      "nix env *"
      "nix flake init *"
      "nix flake new *"
      "nix flake clone *"
      "nix key *"
      "nix print-dev-env *"
      "nix profile install *"
      "nix profile remove *"
      "nix profile rollback *"
      "nix profile upgrade *"
      "nix realisation *"
      "nix registry add *"
      "nix registry remove *"
      "nix registry pin *"
      "nix repl *"
      "nix run *"
      "nix shell *"
      "nix store gc *"
      "nix store optimise *"
      "nix store repair *"
      "nix store verify *"
      "nix store sign *"
      "nix store make-content-addressed *"
      "nix-store --realise *"
      "nix-store -r *"
      "nix-shell *"
      "nix-env *"
    ]
    ++ lib.optionals storeAddAsk [
      "nix store add *"
      "nix store add-file *"
      "nix store add-path *"
    ];

  deny =
    wrapped queryWrappers ["nix *" "nix-*" "nixos-*"]
    # These suffix rules sort after the corresponding allow. Conservatively
    # reject even quoted/repeated settings instead of interpreting shell argv.
    ++ wrapped queryWrappers (map (prefix: "${prefix}*allow-import-from-derivation*") evaluationPrefixes)
    ++ wrapped queryWrappers [
      "nix build --dry-run --no-allow-import-from-derivation *--no-dry-run*"
      "nix flake check --no-build --no-allow-import-from-derivation *--build*"
      "nix-instantiate --parse *--eval*"
    ]
    # Suffix guards sort after their store-query allow. Do not reinterpret
    # store paths through an expression/file, add hash-qualified flake targets
    # or repair. These are command-shape globs, not an argv validator.
    ++ wrapped queryWrappers (lib.concatMap (prefix:
      map (flag: "${prefix}*${flag}*") [
        # A trailing ` *` also matches no argument in OpenCode. Require
        # one character after a separator so the exact-path form stays safe.
        " ?"
        "\t?"
        "\n?"
        "^"
        "$"
        "`"
        "/"
        "%"
        "="
        " --expr"
        " --file"
        " -f"
        " --stdin"
        " --inputs-from"
        " --override-flake"
        " --override-input"
        " --repair"
        " --allow-import-from-derivation"
        "#"
      ])
    storeMetadataPrefixes)
    ++ wrapped queryWrappers (map (prefix: "${prefix}.*") storeMetadataPrefixes)
    ++ wrapped queryWrappers (lib.concatMap (prefix:
      map (flag: "${prefix}*${flag}*") [
        "--realise"
        "--force-realise"
        "--use-output"
        "-r "
        "--delete"
        "--gc"
        "--import"
        "--restore"
        "--repair"
        "--load-db"
        "--register-validity"
        "--serve"
      ])
    storeQueryPrefixes);
}
