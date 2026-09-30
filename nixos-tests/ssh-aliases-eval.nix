{pkgs}: let
  inherit (pkgs) lib;
  inherit (import ./lib/eval-checks.nix {inherit pkgs;}) mkEvalCheck;

  sshAliases = import ../lib/sshAliases.nix;
  hosts = {
    alpha = {
      lanIp = "192.0.2.1";
      wgHomeIp = "198.51.100.1";
      directLinkPeers = [];
    };
    beta = {
      lanIp = "192.0.2.2";
      wgHomeIp = "198.51.100.2";
      directLinkPeers = [];
    };
  };
  rbac = {userCanReach = _user: _host: true;};
  base = {
    inherit lib hosts rbac;
    fromHost = "alpha";
    user = "ops";
    defaultIdentity = "~/.ssh/id";
    defaultPort = 22;
  };

  closed = sshAliases base;
  tunneled = sshAliases (base
    // {
      jumpHostAlias = "vgamma";
      jumpExcludedHost = "beta";
    });
  relaxed = sshAliases (base
    // {
      jumpHostAlias = "vgamma";
      jumpExcludedHost = "gamma";
      allowRelaxedInitrdCheck = true;
    });
  noInitrd = sshAliases (base
    // {
      jumpHostAlias = "vgamma";
      jumpExcludedHost = "gamma";
      enableInitrdAlias = false;
    });
  overridden = sshAliases (base
    // {
      jumpHostAlias = "vgamma";
      jumpExcludedHost = "gamma";
    }
    // {overrides = {beta = {port = 2222;};};});
  cloudHosts =
    hosts
    // {
      alpha = hosts.alpha // {linkAddresses.wg-edge = "10.90.0.2";};
      edge = {
        hostPubkey = "ssh-ed25519 fixture-key";
        management = {
          publicAddress = "192.0.2.10";
          sshPort = 1337;
          link = "wg-edge";
        };
        linkAddresses.wg-edge = "10.90.0.1";
      };
    };
  cloud = sshAliases (base // {hosts = cloudHosts;});
  recoveryOnly = sshAliases (base // {hosts = cloudHosts // {inherit (hosts) alpha;};});
  restricted = sshAliases (base
    // {
      hosts = cloudHosts;
      rbac.userCanReach = _user: host: host != "edge";
    });
  cloudOverrides = sshAliases (base
    // {
      hosts = cloudHosts;
      overrides.edge = {
        port = 22;
        extraOptions = {
          StrictHostKeyChecking = "no";
          UserKnownHostsFile = "~/.ssh/known_hosts";
          ControlMaster = "auto";
        };
      };
    });
  untrusted = builtins.tryEval (builtins.deepSeq (sshAliases (base
    // {
      hosts = cloudHosts // {edge = cloudHosts.edge // {hostPubkey = null;};};
    }))
  true);
in
  mkEvalCheck {
    name = "ssh-aliases-eval";
    resultMessage = "ssh alias tunnel policy and relaxed-check opt-in passed";
    assertions = [
      {
        name = "fail-closed-without-jump";
        assertion = builtins.attrNames closed == ["lbeta" "vbeta"];
        message = "without jumpHostAlias only direct lan/vpn aliases may be generated";
      }
      {
        name = "tunnel-aliases-when-configured";
        assertion = builtins.attrNames tunneled == ["ibeta" "lbeta" "vbeta"];
        message = "a configured jump host enables the initrd alias (tbeta excluded below)";
      }
      {
        name = "excluded-host-keeps-direct";
        assertion = !(tunneled ? "tbeta") && (tunneled ? "lbeta") && (tunneled ? "vbeta");
        message = "jumpExcludedHost must lose its tunnel alias but keep direct aliases";
      }
      {
        name = "host-key-verification-kept-by-default";
        assertion = !(tunneled.ibeta ? StrictHostKeyChecking) && !(tunneled.ibeta ? UserKnownHostsFile);
        message = "the initrd alias must not disable host-key verification unless explicitly allowed";
      }
      {
        name = "relaxed-check-explicit";
        assertion = relaxed.ibeta.StrictHostKeyChecking == "no" && relaxed.ibeta.UserKnownHostsFile == "/dev/null" && relaxed.ibeta.ProxyJump == "vgamma";
        message = "allowRelaxedInitrdCheck must add the bypass settings to the initrd alias";
      }
      {
        name = "initrd-toggle";
        assertion = !(noInitrd ? "ibeta") && (noInitrd ? "tbeta");
        message = "enableInitrdAlias=false must drop only the initrd alias";
      }
      {
        name = "per-target-override";
        assertion = overridden.tbeta.Port == 2222 && overridden.lbeta.Port == 2222;
        message = "per-target port overrides must apply to generated aliases";
      }
      {
        name = "cloud-without-lan";
        assertion = cloud.eedge.HostName == "10.90.0.1" && cloud.pedge.HostName == "192.0.2.10" && !(cloud ? ledge) && !(cloud ? vedge);
        message = "management must use the declared shared link and independent public address without inventing LAN/VPN routes";
      }
      {
        name = "cloud-port-and-identity";
        assertion = cloud.eedge.Port == 1337 && cloud.pedge.Port == 1337 && cloud.eedge.HostKeyAlias == "edge" && cloud.pedge.StrictHostKeyChecking == "yes" && !cloud.pedge.PasswordAuthentication;
        message = "installed port and canonical pinned identity must apply to both management routes";
      }
      {
        name = "runtime-pins-cannot-be-relaxed-by-legacy-overrides";
        assertion = cloudOverrides.pedge.Port == 1337 && cloudOverrides.pedge.StrictHostKeyChecking == "yes" && cloudOverrides.pedge.UserKnownHostsFile == "/dev/null" && cloudOverrides.pedge.GlobalKnownHostsFile == "/etc/ssh/ssh_known_hosts" && cloudOverrides.pedge.ControlMaster == "no";
        message = "management must use the enrolled installed identity and port, without user-file keys or cached unverified sessions";
      }
      {
        name = "independent-recovery-and-rbac";
        assertion = !(recoveryOnly ? eedge) && (recoveryOnly ? pedge) && !(restricted ? pedge) && !(restricted ? eedge) && !untrusted.success;
        message = "public recovery must survive absent link membership, preserve RBAC, and require runtime identity";
      }
    ];
  }
