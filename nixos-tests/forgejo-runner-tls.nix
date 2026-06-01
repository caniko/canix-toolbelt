{pkgs, ...}:
pkgs.testers.nixosTest {
  name = "forgejo-runner-tls";

  nodes.server = {
    config,
    lib,
    pkgs,
    ...
  }: let
    containerEtc = pkgs.runCommand "forgejo-runner-tls-container-etc" {} ''
      mkdir -p "$out/etc"
      cat > "$out/etc/nsswitch.conf" <<'EOF'
      hosts: files dns
      EOF
      cat > "$out/etc/host.conf" <<'EOF'
      multi on
      EOF
      cat > "$out/etc/resolv.conf" <<'EOF'
      nameserver 10.0.2.3
      options edns0
      EOF
      cat > "$out/etc/hosts" <<'EOF'
      127.0.0.1 localhost
      ::1 localhost
      EOF
    '';
    runtime = config.canix-toolbelt.services.forgejoRunner.containerRuntime;
  in {
    imports = [
      ../modules/nixos/services/forgejo-runner.nix
      ../modules/nixos/services/forgejo-runner-container-runtime.nix
    ];

    virtualisation.memorySize = 3072;
    virtualisation.diskSize = 4096;

    virtualisation.podman.enable = true;
    virtualisation.podman.dockerSocket.enable = true;

    services.forgejo = {
      enable = true;
      settings = {
        actions.ENABLED = true;
        repository = {
          ENABLE_PUSH_CREATE_USER = true;
          DEFAULT_PUSH_CREATE_PRIVATE = false;
        };
        server = {
          DOMAIN = "localhost";
          HTTP_PORT = 3000;
          ROOT_URL = "http://localhost:3000/";
        };
        service.DISABLE_REGISTRATION = true;
      };
    };

    environment.systemPackages = with pkgs; [
      config.services.forgejo.package
      gitMinimal
      jq
      openssl
    ];

    canix-toolbelt.services.forgejoRunner.containerRuntime = {
      enable = true;
      actionRuntimePackages = lib.mkAfter [pkgs.strace];
      actionRuntimeExecutables = lib.mkAfter ["strace"];
      extraContainerOptions = [
        "-v ${containerEtc}/etc/nsswitch.conf:/etc/nsswitch.conf:ro"
        "-v ${containerEtc}/etc/host.conf:/etc/host.conf:ro"
        "-v ${containerEtc}/etc/resolv.conf:/etc/resolv.conf:ro"
        "-v ${containerEtc}/etc/hosts:/etc/hosts:ro"
        # Phase 03 selected 3d/no-code-change. Inject an explicit late
        # container env leak so the runtime's SSL_CERT_FILE/NIX_SSL_CERT_FILE
        # overrides must be true last-wins options.
        "-e SSL_CERT_FILE=/nix/var/nix/profiles/default/etc/ssl/certs/ca-bundle.crt"
        "-e NIX_SSL_CERT_FILE=/nix/var/nix/profiles/default/etc/ssl/certs/ca-bundle.crt"
      ];
      extraValidVolumes = [
        "${containerEtc}/etc/nsswitch.conf"
        "${containerEtc}/etc/host.conf"
        "${containerEtc}/etc/resolv.conf"
        "${containerEtc}/etc/hosts"
      ];
    };

    services.forgejo.runner.instances.test = {
      enable = true;
      url = "http://localhost:3000";
      registrationTokenFile = "/var/lib/forgejo/runner_token";
      labels = [
        # Match atlas' docker-scheme label shape so the runtime container
        # options and locally loaded runner image are exercised.
        "test:${runtime.imageRef}"
      ];
      settings = {
        log.level = "info";
        runner = {
          name = "test";
          capacity = 1;
        };
        container = {
          # Use the VM's host network so the hermetic HTTPS probe can reach a
          # local TLS server while still exercising the atlas docker-scheme
          # runtime options.
          network = "host";
          privileged = false;
          force_pull = false;
          options = runtime.containerOptions;
          valid_volumes = runtime.validVolumes;
        };
      };
    };

    # Phase 03 selected 3d/no-code-change in:
    # /data/nvme0/can/Projects/canix/docs/planning/forgejo-runner-tls-fix/02-diag-outcome.md
    # Simulate a systemd-unit environment leak source. The current runtime must
    # neutralize it by making the container SSL_CERT_FILE/NIX_SSL_CERT_FILE
    # point at /canix-forgejo-action-certs instead of this host profile path.
    systemd.services."forgejo-runner@test" = {
      wantedBy = lib.mkForce [];
      environment = {
        SSL_CERT_FILE = "/nix/var/nix/profiles/default/etc/ssl/certs/ca-bundle.crt";
        NIX_SSL_CERT_FILE = "/nix/var/nix/profiles/default/etc/ssl/certs/ca-bundle.crt";
      };
    };
  };

  testScript = ''
    import json
    import shlex

    leaked_path = "/nix/var/nix/profiles/default/"

    start_all()

    server.wait_for_unit("forgejo.service")
    server.wait_for_open_port(3000)
    server.wait_until_succeeds(
        'test "$(systemctl show -P ActiveState forgejo-runner-image-load.service)" = inactive && '
        + 'test "$(systemctl show -P Result forgejo-runner-image-load.service)" = success'
    )
    server.succeed("podman image exists localhost/canix-runner:local")
    server.succeed("podman image exists localhost/canix-nix-runner:local")
    server.succeed("podman run --rm localhost/canix-runner:local sudo sh -c 'test \"$(id -u)\" = 0'")
    server.succeed("podman run --rm localhost/canix-nix-runner:local sudo sh -c 'test \"$(id -u)\" = 0'")
    server.succeed("curl --fail http://localhost:3000/")
    server.succeed(
        "mkdir -p /tmp/local-https && "
        + "openssl req -x509 -newkey rsa:2048 -days 1 -nodes "
        + "-subj /CN=forgejo-runner-tls.localhost "
        + "-addext subjectAltName=DNS:forgejo-runner-tls.localhost "
        + "-keyout /tmp/local-https/key.pem -out /tmp/local-https/cert.pem >/tmp/local-https/openssl.log 2>&1"
    )
    server.succeed(
        "systemd-run --unit=forgejo-runner-tls-https --collect "
        + "--property=StandardOutput=append:/tmp/local-https/server.log "
        + "--property=StandardError=append:/tmp/local-https/server.log "
        + "openssl s_server -quiet -www -accept 8443 "
        + "-cert /tmp/local-https/cert.pem -key /tmp/local-https/key.pem"
    )
    server.wait_for_open_port(8443, timeout=30)

    server.succeed(
        "su -l forgejo -c 'GITEA_WORK_DIR=/var/lib/forgejo forgejo admin user create "
        + "--username test --password totallysafe --email test@localhost --must-change-password=false'"
    )

    api_token = server.succeed(
        "curl --fail -X POST http://test:totallysafe@localhost:3000/api/v1/users/test/tokens "
        + "-H 'Accept: application/json' -H 'Content-Type: application/json' "
        + "-d '{\"name\":\"token\",\"scopes\":[\"all\"]}' | jq -r '.sha1'"
    ).strip()

    server.succeed(
        "curl --fail -X POST http://localhost:3000/api/v1/user/repos "
        + "-H 'Accept: application/json' -H 'Content-Type: application/json' "
        + f"-H 'Authorization: token {api_token}' "
        + "-d '{\"auto_init\":false,\"name\":\"repo\",\"private\":false}'"
    )

    server.succeed(
        "curl --fail -X PATCH http://localhost:3000/api/v1/repos/test/repo "
        + "-H 'Accept: application/json' -H 'Content-Type: application/json' "
        + f"-H 'Authorization: token {api_token}' "
        + "-d '{\"has_actions\":true}'"
    )

    server.succeed(
        "su -l forgejo -c 'GITEA_WORK_DIR=/var/lib/forgejo forgejo actions generate-runner-token' "
        + "> /var/lib/forgejo/runner_token"
    )
    server.systemctl("start forgejo-runner@test.service")
    server.wait_for_unit("forgejo-runner@test.service")
    server.wait_until_succeeds(
        "journalctl -o cat -u forgejo-runner@test.service | grep -q 'Runner registered successfully'",
        timeout=60,
    )

    # Keep the workflow container alive while the host probes it. Running the
    # diagnostic via podman exec avoids racing Forgejo runner's own step
    # scheduling while still testing the live job container environment.
    workflow = r"""
    on:
      push:
    jobs:
      tls-diag:
        runs-on: test
        steps:
          - name: hold container for tls diagnostic
            run: |
              set -eux
              sleep 45
    """

    quoted_workflow = shlex.quote(workflow)
    server.succeed("mkdir -p /tmp/repo/.forgejo/workflows")
    server.succeed("git -C /tmp/repo init --initial-branch=main")
    server.succeed("git -C /tmp/repo config user.email test@localhost")
    server.succeed("git -C /tmp/repo config user.name test")
    server.succeed(f"printf %s {quoted_workflow} > /tmp/repo/.forgejo/workflows/tls-diag.yml")
    server.succeed("git -C /tmp/repo add .")
    server.succeed("git -C /tmp/repo commit -m 'Add TLS diagnostic workflow'")
    server.succeed("git -C /tmp/repo remote add origin http://test:totallysafe@localhost:3000/test/repo.git")
    server.succeed("git -C /tmp/repo push origin main")

    def find_job_container(_):
        container_id = server.succeed(
            "podman ps --filter label=com.github.actions.job=tls-diag --format '{{.ID}}' | head -1"
        ).strip()
        if not container_id:
            container_id = server.succeed(
                "podman ps --format '{{.ID}} {{.Image}} {{.Command}}' "
                + "| awk '/canix-runner|sleep 120|bash/ {print $1; exit}'"
            ).strip()
        return bool(container_id)

    with server.nested("Waiting for the diagnostic job container"):
        retry(find_job_container, 90)

    container_id = server.succeed(
        "podman ps --filter label=com.github.actions.job=tls-diag --format '{{.ID}}' | head -1"
    ).strip()
    if not container_id:
        container_id = server.succeed(
            "podman ps --format '{{.ID}} {{.Image}} {{.Command}}' "
            + "| awk '/canix-runner|sleep 120|bash/ {print $1; exit}'"
        ).strip()
    assert container_id, "no job container running"

    def assert_env_values_clean(env_dump, source):
        offenders = []
        for line in env_dump.splitlines():
            key, separator, value = line.partition("=")
            if separator != "=":
                continue
            # The action runtime intentionally exposes profile bin directories
            # in PATH. The TLS invariant is that no CA-related env value keeps
            # the host profile certificate path.
            if key == "PATH":
                continue
            if leaked_path in value:
                offenders.append(line)
        assert not offenders, (
            f"leaked CA path found in {source} env values:\n"
            + "\n".join(offenders)
            + f"\n\nfull {source} env:\n{env_dump}"
        )

    env_dump = server.succeed(f"podman exec {container_id} env")
    assert_env_values_clean(env_dump, "container")

    inspect_env_json = server.succeed(
        f"podman inspect {container_id} --format '{{{{json .Config.Env}}}}'"
    ).strip()
    inspect_env = "\n".join(json.loads(inspect_env_json))
    assert_env_values_clean(inspect_env, "podman Config.Env")

    gitconfig_dump = server.succeed(
        f"podman exec {container_id} git config --list --show-origin || true"
    )
    assert leaked_path not in gitconfig_dump, (
        f"leaked CA path found in container git config:\n{gitconfig_dump}"
    )

    server.succeed(f"podman exec {container_id} mkdir -p /tmp")
    server.succeed(f"podman cp /tmp/local-https/cert.pem {container_id}:/tmp/local-https-ca.pem")
    curl_probe = shlex.quote(
        "mkdir -p /tmp; "
        + "timeout 30s strace -f -e openat -o /tmp/curl-openat.log "
        + "curl --cacert /tmp/local-https-ca.pem --resolve forgejo-runner-tls.localhost:8443:127.0.0.1 "
        + "--connect-timeout 10 --max-time 20 -fsS https://forgejo-runner-tls.localhost:8443/ "
        + ">/tmp/curl-body.txt 2>/tmp/curl-stderr.txt; "
        + "echo $? > /tmp/curl-status.txt"
    )
    server.succeed(f"podman exec {container_id} sh -c {curl_probe}")

    curl_status = server.succeed(f"podman exec {container_id} cat /tmp/curl-status.txt").strip()
    curl_stderr = server.succeed(f"podman exec {container_id} cat /tmp/curl-stderr.txt || true")
    curl_openat_dump = server.succeed(f"podman exec {container_id} cat /tmp/curl-openat.log")
    assert curl_status == "0", (
        f"https curl failed with status {curl_status}\n"
        + f"stderr:\n{curl_stderr}\n"
        + f"openat trace:\n{curl_openat_dump}"
    )
    assert leaked_path not in curl_openat_dump, (
        f"leaked CA path found in libcurl openat trace:\n{curl_openat_dump}"
    )

    git_probe = shlex.quote(
        "timeout 45s git ls-remote http://localhost:3000/test/repo.git "
        + ">/tmp/git-ls-remote.txt 2>/tmp/git-ls-remote.err; "
        + "echo $? > /tmp/git-ls-remote.status"
    )
    server.succeed(f"podman exec {container_id} sh -c {git_probe}")

    git_status = server.succeed(f"podman exec {container_id} cat /tmp/git-ls-remote.status").strip()
    git_stderr = server.succeed(f"podman exec {container_id} cat /tmp/git-ls-remote.err || true")
    git_remote = server.succeed(f"podman exec {container_id} cat /tmp/git-ls-remote.txt || true")
    assert git_status == "0", (
        f"git ls-remote failed with status {git_status}\n"
        + f"stderr:\n{git_stderr}\n"
        + f"stdout:\n{git_remote}"
    )
    assert git_remote, "git ls-remote returned no refs"

    def poll_workflow_action_status(_):
        response = server.succeed(
            "curl --fail http://localhost:3000/api/v1/repos/test/repo/actions/tasks"
        )
        runs = json.loads(response).get("workflow_runs", [])
        status = runs[0].get("status") if runs else "missing"
        server.log(f"Workflow status: {status}")
        if status == "failure":
            raise Exception("Workflow failed")
        return status == "success"

    with server.nested("Waiting for the workflow run to finish"):
        retry(poll_workflow_action_status, 90)
  '';
}
