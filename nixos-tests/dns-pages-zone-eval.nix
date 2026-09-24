{
  inputs,
  pkgs,
}: let
  inherit (pkgs) lib;
  inherit (import ./lib/eval-checks.nix {inherit pkgs;}) mkEvalCheck;

  # The module under test also wires users/systemd/services; stub those
  # option roots so this stays a pure evaluation test.
  stubs = {lib, ...}: {
    options = {
      assertions = lib.mkOption {
        type = lib.types.listOf lib.types.attrs;
        default = [];
      };
      users = lib.mkOption {
        type = lib.types.attrs;
        default = {};
      };
      systemd = lib.mkOption {
        type = lib.types.attrs;
        default = {};
      };
      services = lib.mkOption {
        type = lib.types.attrs;
        default = {};
      };
    };
  };

  site = {
    subdomain = "myproject";
    repository = "other-owner/myproject";
    cnameTarget = "other-owner.codeberg.page";
  };

  evalPages = extra:
    (lib.evalModules {
      modules = [
        stubs
        ../modules/nixos/services/dns-octodns-cloudflare.nix
        {
          _module.args = {inherit inputs pkgs;};
          canix-toolbelt.dns.enable = true;
          canix-toolbelt.dns.zones."example.test" = {records = [];};
        }
        extra
      ];
    })
    .config;

  failedMessages = cfg: builtins.map (a: a.message) (builtins.filter (a: !a.assertion) cfg.assertions);

  missingZone = evalPages {canix-toolbelt.dns.codebergPagesSites = [site];};
  unknownZone = evalPages {
    canix-toolbelt.dns.codebergPagesSites = [site];
    canix-toolbelt.dns.codebergPagesZone = "other.test";
  };
  disabledOk = evalPages {
    canix-toolbelt.dns.codebergPagesSites = [site];
    canix-toolbelt.dns.autoSynthesizeCodebergPagesCnames = false;
  };
  valid = evalPages {
    canix-toolbelt.dns.codebergPagesSites = [site];
    canix-toolbelt.dns.codebergPagesZone = "example.test";
  };
  validRecord = valid.canix-toolbelt.dns.dnsConfig.extraConfig.zones."example.test".myproject.cname or null;
in
  mkEvalCheck {
    name = "dns-pages-zone-eval";
    resultMessage = "codeberg pages zone explicitness and target pass-through passed";
    assertions = [
      {
        name = "missing-zone-fails";
        assertion = failedMessages missingZone == ["codebergPagesZone must be set explicitly when autoSynthesizeCodebergPagesCnames is enabled with a non-empty codebergPagesSites registry (no implicit first-zone fallback)"];
        message = "synthesis with sites but no zone must fail instead of guessing the first declared zone";
      }
      {
        name = "unknown-zone-fails";
        assertion = failedMessages unknownZone == ["codebergPagesZone 'other.test' is not a declared zone (example.test); refusing to synthesize Pages records into nowhere"];
        message = "an explicit zone outside the declared zones must fail";
      }
      {
        name = "disabled-synthesis-ok";
        assertion = failedMessages disabledOk == [];
        message = "disabled synthesis with sites and no zone must not raise pages errors";
      }
      {
        name = "valid-config-ok";
        assertion = failedMessages valid == [];
        message = "an explicit declared zone with sites must not raise pages errors";
      }
      {
        name = "explicit-target-unrelated-owner";
        assertion = validRecord != null && validRecord.data == "other-owner.codeberg.page";
        message = "the synthesized record must use the explicit cnameTarget verbatim, never derived from the repository owner";
      }
    ];
  }
