{
  config,
  lib,
  pkgs,
  utils,
  ...
}: let
  inherit (builtins) attrValues baseNameOf elem isAttrs isList;
  inherit
    (lib)
    attrByPath
    concatMap
    concatStringsSep
    escapeShellArg
    filterAttrs
    foldl'
    hasPrefix
    mkIf
    mkForce
    mkMerge
    mapAttrs'
    nameValuePair
    unique
    ;

  diskoDevices = attrByPath ["disko" "devices"] {} config;
  diskoBcache = diskoDevices.bcache or {};

  collectBcacheMembers = value:
    if isAttrs value
    then let
      filtered = filterAttrs (name: _: !hasPrefix "_" name) value;
      self =
        if filtered ? type && filtered ? set && filtered ? device && elem filtered.type ["bcache_cache" "bcache_backing"]
        then [
          {
            inherit (filtered) device set type;
          }
        ]
        else [];
      children = concatMap collectBcacheMembers (attrValues filtered);
    in
      self ++ children
    else if isList value
    then concatMap collectBcacheMembers value
    else [];

  bcacheMembers = foldl' (
    acc: member:
      acc
      // {
        "${member.set}" = let
          current =
            acc.${
              member.set
            } or {
              backing = [];
              cache = [];
            };
          key =
            if member.type == "bcache_cache"
            then "cache"
            else "backing";
        in
          current
          // {
            "${key}" = current.${key} ++ [member.device];
          };
      }
  ) {} (collectBcacheMembers diskoDevices);

  bootRelevantBcache =
    filterAttrs (
      _: bcacheCfg:
        lib.any (
          fs: fs.device == bcacheCfg.device && utils.fsNeededForBoot fs
        ) (attrValues config.fileSystems)
    )
    diskoBcache;

  mkRegistrationLoop = role: devices:
    concatStringsSep "\n" (map (device: ''
        wait_for_block ${escapeShellArg device}
        register_bcache_member ${escapeShellArg role} ${escapeShellArg device}
      '')
      devices);
in {
  config = mkMerge [
    {
      assertions =
        lib.mapAttrsToList (
          name: _: let
            members =
              bcacheMembers.${
                name
              } or {
                backing = [];
                cache = [];
              };
          in {
            assertion = members.cache != [] && members.backing != [];
            message = ''
              boot-critical disko bcache set "${name}" must have both cache and backing members.
            '';
          }
        )
        bootRelevantBcache;
    }
    (mkIf (bootRelevantBcache != {}) {
      boot.bcache.enable = mkForce false;

      environment.systemPackages = [pkgs.bcache-tools];

      boot.initrd.systemd.initrdBin = [
        pkgs.bcache-tools
        pkgs.btrfs-progs
        pkgs.coreutils
        pkgs.gnugrep
        pkgs.kmod
        config.boot.initrd.systemd.package
        pkgs.util-linux
      ];

      boot.initrd.services.udev = {
        packages = [pkgs.bcache-tools];
        binPackages = [pkgs.bcache-tools];
      };

      boot.initrd.systemd.services =
        mapAttrs' (
          name: bcacheCfg: let
            members =
              bcacheMembers.${
                name
              } or {
                backing = [];
                cache = [];
              };
            cacheDevices = unique members.cache;
            backingDevices = unique members.backing;
            bcacheDevice = bcacheCfg.device;
            bcacheBasename = baseNameOf bcacheDevice;
          in
            nameValuePair "disko-bcache-${name}" {
              description = "Assemble disko bcache set ${name} in initrd";
              wantedBy = ["initrd-root-device.target"];
              before = ["initrd-root-device.target"];
              after = [
                "systemd-modules-load.service"
                "systemd-udevd.service"
                "systemd-udev-trigger.service"
              ];
              wants = [
                "systemd-udevd.service"
                "systemd-udev-trigger.service"
              ];
              unitConfig.DefaultDependencies = "no";
              serviceConfig = {
                Type = "oneshot";
                RemainAfterExit = true;
                StandardError = "journal+console";
                StandardOutput = "journal+console";
                TimeoutStartSec = "2min";
              };
              script = ''
                set -eu

                echo "disko-bcache-initrd: assembling ${name} (${bcacheDevice})"

                dump_bcache_diagnostics() {
                  echo "disko-bcache-initrd: diagnostics for ${name}" >&2
                  echo "disko-bcache-initrd: expected cache devices: ${concatStringsSep " " cacheDevices}" >&2
                  echo "disko-bcache-initrd: expected backing devices: ${concatStringsSep " " backingDevices}" >&2
                  echo "disko-bcache-initrd: expected bcache device: ${bcacheDevice}" >&2

                  echo "disko-bcache-initrd: /dev/bcache*:" >&2
                  ls -l /dev/bcache* >&2 2>/dev/null || true

                  echo "disko-bcache-initrd: /dev/disk/by-partlabel:" >&2
                  ls -l /dev/disk/by-partlabel >&2 2>/dev/null || true

                  echo "disko-bcache-initrd: /sys/fs/bcache:" >&2
                  ls -la /sys/fs/bcache >&2 2>/dev/null || true

                  echo "disko-bcache-initrd: /sys/block bcache entries:" >&2
                  for path in /sys/block/*/bcache /sys/class/block/*/bcache; do
                    if [ -e "$path" ]; then
                      echo "disko-bcache-initrd: $path" >&2
                      ls -la "$path" >&2 2>/dev/null || true
                    fi
                  done

                  for dev in ${concatStringsSep " " (map escapeShellArg (cacheDevices ++ backingDevices))}; do
                    if [ -e "$dev" ]; then
                      local resolved
                      resolved="$(readlink -f "$dev")"
                      echo "disko-bcache-initrd: bcache-super-show $dev -> $resolved" >&2
                      bcache-super-show "$resolved" >&2 2>/dev/null || true
                    else
                      echo "disko-bcache-initrd: missing configured member $dev" >&2
                    fi
                  done
                }

                trap dump_bcache_diagnostics EXIT

                wait_for_block() {
                  local dev="$1"
                  local i

                  for i in $(seq 1 120); do
                    if [ -b "$dev" ]; then
                      return 0
                    fi
                    udevadm settle --timeout=1 || true
                    sleep 1
                  done

                  echo "disko-bcache-initrd: timed out waiting for $dev" >&2
                  return 1
                }

                wait_for_path() {
                  local path="$1"
                  local i

                  for i in $(seq 1 30); do
                    if [ -e "$path" ]; then
                      return 0
                    fi
                    sleep 1
                  done

                  echo "disko-bcache-initrd: timed out waiting for $path" >&2
                  return 1
                }

                wait_for_cache_mode() {
                  local path="$1"
                  local expected="$2"
                  local i

                  for i in $(seq 1 30); do
                    if grep -Fq "[$expected]" "$path"; then
                      return 0
                    fi
                    sleep 1
                  done

                  echo "disko-bcache-initrd: active cache_mode did not become $expected: $(cat "$path")" >&2
                  return 1
                }

                wait_for_member_registration() {
                  local block_name="$1"
                  local i

                  for i in $(seq 1 30); do
                    if [ -e "/sys/class/block/$block_name/bcache" ] || [ -e "/sys/block/$block_name/bcache" ] || [ -b ${escapeShellArg bcacheDevice} ]; then
                      return 0
                    fi
                    udevadm settle --timeout=1 || true
                    sleep 1
                  done

                  echo "disko-bcache-initrd: $block_name did not register with bcache" >&2
                  return 1
                }

                wait_for_cache_registration() {
                  local i

                  for i in $(seq 1 30); do
                    for cset in /sys/fs/bcache/*-*-*; do
                      if [ -d "$cset" ] && ls "$cset"/cache* >/dev/null 2>&1; then
                        return 0
                      fi
                    done
                    udevadm settle --timeout=1 || true
                    sleep 1
                  done

                  echo "disko-bcache-initrd: cache set did not register with bcache" >&2
                  return 1
                }

                register_bcache_member() {
                  local role="$1"
                  local dev="$2"
                  local resolved
                  local block_name

                  resolved="$(readlink -f "$dev")"
                  block_name="$(basename "$resolved")"
                  echo "disko-bcache-initrd: member $dev resolves to $resolved ($block_name)"
                  if ! bcache-super-show "$resolved"; then
                    echo "disko-bcache-initrd: $resolved does not contain a bcache superblock" >&2
                    return 1
                  fi

                  if [ -e "/sys/class/block/$block_name/bcache" ] || [ -e "/sys/block/$block_name/bcache" ]; then
                    echo "disko-bcache-initrd: $resolved already registered"
                    return 0
                  fi

                  echo "disko-bcache-initrd: registering $resolved"
                  if ! printf '%s\n' "$resolved" > /sys/fs/bcache/register; then
                    if [ "$role" = cache ] && wait_for_cache_registration; then
                      echo "disko-bcache-initrd: cache set registered despite register write failure"
                      return 0
                    fi
                    if [ -e "/sys/class/block/$block_name/bcache" ] || [ -e "/sys/block/$block_name/bcache" ]; then
                      echo "disko-bcache-initrd: $resolved registered despite register write failure"
                      return 0
                    fi
                    echo "disko-bcache-initrd: failed to register $resolved" >&2
                    return 1
                  fi
                  if [ "$role" = cache ]; then
                    wait_for_cache_registration
                  else
                    wait_for_member_registration "$block_name"
                  fi
                }

                modprobe bcache
                wait_for_path /sys/fs/bcache/register

                ${mkRegistrationLoop "cache" cacheDevices}
                ${mkRegistrationLoop "backing" backingDevices}

                udevadm settle --timeout=10 || true

                if [ ! -b ${escapeShellArg bcacheDevice} ]; then
                  echo "disko-bcache-initrd: waiting for ${bcacheDevice}" >&2
                  wait_for_block ${escapeShellArg bcacheDevice}
                fi

                if [ ! -b ${escapeShellArg bcacheDevice} ]; then
                  echo "disko-bcache-initrd: ${bcacheDevice} did not appear" >&2
                  exit 1
                fi

                if [ -e /sys/block/${escapeShellArg bcacheBasename}/bcache/cache_mode ]; then
                  echo "disko-bcache-initrd: setting cache_mode to ${bcacheCfg.cacheMode}"
                  echo ${escapeShellArg bcacheCfg.cacheMode} > /sys/block/${escapeShellArg bcacheBasename}/bcache/cache_mode
                  wait_for_cache_mode /sys/block/${escapeShellArg bcacheBasename}/bcache/cache_mode ${escapeShellArg bcacheCfg.cacheMode}
                fi

                echo "disko-bcache-initrd: ${bcacheDevice} is ready"
                trap - EXIT
              '';
            }
        )
        bootRelevantBcache;
    })
  ];
}
