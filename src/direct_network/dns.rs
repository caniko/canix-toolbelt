//! Authenticate the resolver unit, its cgroup generation, and its four sockets.

use super::{Policy, Result, args, invalid, output};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::os::unix::fs::MetadataExt;
use std::path::Path;

const UNIT: &str = "direct-network-dns.service";
const PORT: u16 = 5354;

/// An observation binds process identity to the currently enrolled kernel objects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Identity {
    pid: u32,
    starttime: u64,
    cgroup_inode: u64,
    net_namespace: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Observation {
    pub identity: Identity,
    sockets: [u64; 4],
}

fn bounded_text(path: &Path, limit: u64) -> Result<String> {
    let mut text = String::new();
    fs::File::open(path)?
        .take(limit + 1)
        .read_to_string(&mut text)?;
    if text.len() as u64 > limit {
        return Err(invalid("resolver evidence exceeds bound"));
    }
    Ok(text)
}

fn starttime(stat: &str, pid: u32) -> Result<u64> {
    let (prefix, rest) = stat
        .rsplit_once(')')
        .ok_or_else(|| invalid("invalid resolver process stat"))?;
    if !prefix.starts_with(&format!("{pid} (")) {
        return Err(invalid("resolver process PID mismatch"));
    }
    rest.split_whitespace()
        .nth(19)
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| invalid("invalid resolver process start time"))
}

fn unit_pid(properties: &str, cgroup: &str) -> Result<u32> {
    let mut fields = BTreeMap::new();
    for line in properties.lines() {
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| invalid("invalid resolver unit properties"))?;
        if fields.insert(key, value).is_some() {
            return Err(invalid("duplicate resolver unit property"));
        }
    }
    if fields.get("ActiveState") != Some(&"active")
        || fields.get("SubState") != Some(&"running")
        || fields.get("ControlGroup").copied() != Some(format!("/{cgroup}").as_str())
    {
        return Err(invalid(
            "resolver unit is not running in its enrolled cgroup",
        ));
    }
    fields
        .get("MainPID")
        .and_then(|v| v.parse().ok())
        .filter(|pid| *pid > 0)
        .ok_or_else(|| invalid("resolver unit has no main process"))
}

fn identity(
    proc_root: &Path,
    cgroup_root: &Path,
    cgroup: &str,
    pid: u32,
    enrolled: u64,
) -> Result<Identity> {
    let process = proc_root.join(pid.to_string());
    let membership = bounded_text(&process.join("cgroup"), 65536)?;
    if membership
        .lines()
        .filter(|line| line.starts_with("0::"))
        .collect::<Vec<_>>()
        != [format!("0::/{cgroup}")]
    {
        return Err(invalid("resolver main process cgroup mismatch"));
    }
    let cg = fs::metadata(cgroup_root.join(cgroup))?;
    if !cg.is_dir() || cg.ino() != enrolled {
        return Err(invalid("resolver cgroup generation changed"));
    }
    let namespace = fs::metadata(process.join("ns/net"))?;
    let local_namespace = fs::metadata(proc_root.join("self/ns/net"))?;
    if (namespace.dev(), namespace.ino()) != (local_namespace.dev(), local_namespace.ino()) {
        return Err(invalid("resolver is outside the host network namespace"));
    }
    Ok(Identity {
        pid,
        starttime: starttime(&bounded_text(&process.join("stat"), 65536)?, pid)?,
        cgroup_inode: cg.ino(),
        net_namespace: namespace.ino(),
    })
}

fn socket_inodes(process: &Path) -> Result<BTreeSet<u64>> {
    let mut sockets = BTreeSet::new();
    for (count, entry) in fs::read_dir(process.join("fd"))?.enumerate() {
        if count >= 4096 {
            return Err(invalid("resolver descriptor evidence exceeds bound"));
        }
        let entry = entry?;
        let link = match fs::read_link(entry.path()) {
            Ok(link) => link,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        if let Some(socket) = link
            .to_str()
            .and_then(|s| s.strip_prefix("socket:["))
            .and_then(|s| s.strip_suffix(']'))
        {
            sockets.insert(
                socket
                    .parse()
                    .map_err(|_| invalid("invalid resolver socket inode"))?,
            );
        }
    }
    Ok(sockets)
}

fn endpoint(text: &str, v6: bool) -> Result<(IpAddr, u16)> {
    let (address, port) = text
        .split_once(':')
        .ok_or_else(|| invalid("invalid proc socket endpoint"))?;
    let port = u16::from_str_radix(port, 16)?;
    if !address.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid("invalid proc address encoding"));
    }
    let ip = if v6 {
        if address.len() != 32 {
            return Err(invalid("invalid proc IPv6 socket endpoint"));
        }
        let mut octets = [0; 16];
        for (i, chunk) in octets.chunks_mut(4).enumerate() {
            chunk.copy_from_slice(
                &u32::from_str_radix(&address[i * 8..i * 8 + 8], 16)?.to_ne_bytes(),
            );
        }
        IpAddr::V6(Ipv6Addr::from(octets))
    } else {
        if address.len() != 8 {
            return Err(invalid("invalid proc IPv4 socket endpoint"));
        }
        IpAddr::V4(Ipv4Addr::from(
            u32::from_str_radix(address, 16)?.to_ne_bytes(),
        ))
    };
    Ok((ip, port))
}

fn listener_inode(table: &str, v6: bool, udp: bool, owned: &BTreeSet<u64>) -> Result<u64> {
    let required = if v6 {
        IpAddr::V6(Ipv6Addr::LOCALHOST)
    } else {
        IpAddr::V4(Ipv4Addr::new(127, 0, 0, 54))
    };
    let mut listeners = Vec::new();
    for row in table.lines().skip(1) {
        let fields = row.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 10 {
            return Err(invalid("truncated proc socket row"));
        }
        let local = endpoint(fields[1], v6)?;
        // All four tables are observed: an IPv4-mapped socket in udp6 can
        // compete with the literal IPv4 listener in udp. Wildcards can compete
        // in either family too. Reject aliases before filtering exact listeners
        // or MainPID ownership; established TCP connections are not listeners.
        let normalized_ip = match local.0 {
            IpAddr::V6(ip) => ip.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(local.0),
            IpAddr::V4(_) => local.0,
        };
        if local.1 == PORT
            && local.0 != required
            && (normalized_ip.is_unspecified()
                || normalized_ip == IpAddr::V4(Ipv4Addr::new(127, 0, 0, 54)))
            && (udp || fields[3] == "0A")
        {
            return Err(invalid("competing wildcard or mapped resolver socket"));
        }
        if local != (required, PORT) {
            continue;
        }
        if udp && fields[3] != "07" {
            return Err(invalid(
                "competing connected or incompatible UDP resolver socket",
            ));
        }
        if !udp && fields[3] != "0A" {
            continue;
        }
        let peer = endpoint(fields[2], v6)?;
        if !peer.0.is_unspecified() || peer.1 != 0 {
            return Err(invalid("connected resolver listener"));
        }
        listeners.push(fields[9].parse::<u64>()?);
    }
    // Inspect every matching row before ownership filtering: an owned listener
    // must not hide an unrelated SO_REUSEPORT listener at the same endpoint.
    if listeners.len() != 1 || listeners[0] == 0 || !owned.contains(&listeners[0]) {
        return Err(invalid("missing, ambiguous, or foreign resolver listener"));
    }
    Ok(listeners[0])
}

fn capture(
    properties: &str,
    policy: &Policy,
    cgroups: &BTreeMap<String, u64>,
    proc_root: &Path,
    cgroup_root: &Path,
) -> Result<Observation> {
    let cgroup = policy
        .dns_cgroup
        .as_deref()
        .ok_or_else(|| invalid("resolver cgroup is not configured"))?;
    let enrolled = *cgroups
        .get(cgroup)
        .ok_or_else(|| invalid("resolver cgroup is not enrolled"))?;
    let pid = unit_pid(properties, cgroup)?;
    let before = identity(proc_root, cgroup_root, cgroup, pid, enrolled)?;
    let process = proc_root.join(pid.to_string());
    let owned = socket_inodes(&process)?;
    let mut sockets = [0; 4];
    for (i, (name, v6, udp)) in [
        ("tcp", false, false),
        ("tcp6", true, false),
        ("udp", false, true),
        ("udp6", true, true),
    ]
    .into_iter()
    .enumerate()
    {
        sockets[i] = listener_inode(
            &bounded_text(&process.join("net").join(name), 8 * 1024 * 1024)?,
            v6,
            udp,
            &owned,
        )?;
    }
    let retained = socket_inodes(&process)?;
    if sockets.iter().collect::<BTreeSet<_>>().len() != 4
        || !sockets.iter().all(|i| retained.contains(i))
    {
        return Err(invalid("resolver listener ownership changed"));
    }
    if identity(proc_root, cgroup_root, cgroup, pid, enrolled)? != before {
        return Err(invalid("resolver process identity changed"));
    }
    Ok(Observation {
        identity: before,
        sockets,
    })
}

#[cfg(test)]
pub(super) fn test_observation(pid: u32) -> Observation {
    Observation {
        identity: Identity {
            pid,
            starttime: 987,
            cgroup_inode: 123,
            net_namespace: 456,
        },
        sockets: [1, 2, 3, 4],
    }
}

fn properties(policy: &Policy) -> Result<String> {
    output(
        &policy.systemctl,
        &args(&[
            "show",
            UNIT,
            "--property=ActiveState,SubState,MainPID,ControlGroup",
        ]),
        None,
    )
}

pub(super) fn observe(policy: &Policy, cgroups: &BTreeMap<String, u64>) -> Result<Observation> {
    let first = capture(
        &properties(policy)?,
        policy,
        cgroups,
        Path::new("/proc"),
        Path::new("/sys/fs/cgroup"),
    )?;
    let second = capture(
        &properties(policy)?,
        policy,
        cgroups,
        Path::new("/proc"),
        Path::new("/sys/fs/cgroup"),
    )?;
    if first != second {
        return Err(invalid("resolver authenticated snapshot changed"));
    }
    Ok(second)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn row(v6: bool, udp: bool, inode: u64) -> String {
        row_at(
            if v6 {
                IpAddr::V6(Ipv6Addr::LOCALHOST)
            } else {
                IpAddr::V4(Ipv4Addr::new(127, 0, 0, 54))
            },
            udp,
            inode,
        )
    }

    fn row_at(address: IpAddr, udp: bool, inode: u64) -> String {
        let v6 = address.is_ipv6();
        let ip = match address {
            IpAddr::V6(address) => address
                .octets()
                .chunks(4)
                .map(|s| format!("{:08X}", u32::from_ne_bytes(s.try_into().unwrap())))
                .collect::<String>(),
            IpAddr::V4(address) => format!("{:08X}", u32::from_ne_bytes(address.octets())),
        };
        format!(
            "header\n0: {ip}:14EA {}:0000 {} 00000000:00000000 00:00000000 00000000 0 0 {inode} 0\n",
            if v6 {
                "00000000000000000000000000000000"
            } else {
                "00000000"
            },
            if udp { "07" } else { "0A" }
        )
    }

    #[test]
    fn exact_main_process_listeners_reject_missing_foreign_and_reuseport_sockets() {
        let owned = BTreeSet::from([123]);
        for v6 in [false, true] {
            for udp in [false, true] {
                let valid = row(v6, udp, 123);
                assert_eq!(listener_inode(&valid, v6, udp, &owned).unwrap(), 123);
                assert!(listener_inode("header\n", v6, udp, &owned).is_err());
                assert!(listener_inode(&row(v6, udp, 124), v6, udp, &owned).is_err());
                assert!(
                    listener_inode(
                        &(valid.clone() + row(v6, udp, 124).lines().nth(1).unwrap() + "\n"),
                        v6,
                        udp,
                        &owned
                    )
                    .is_err()
                );
                assert!(listener_inode(&row(v6, udp, 0), v6, udp, &owned).is_err());
                let wrong_state = valid.replace(if udp { " 07 " } else { " 0A " }, " 01 ");
                assert!(listener_inode(&wrong_state, v6, udp, &owned).is_err());
                let wildcard = valid.replace(
                    &valid
                        .lines()
                        .nth(1)
                        .unwrap()
                        .split_whitespace()
                        .nth(1)
                        .unwrap()[..if v6 { 32 } else { 8 }],
                    if v6 {
                        "00000000000000000000000000000000"
                    } else {
                        "00000000"
                    },
                );
                assert!(listener_inode(&wildcard, v6, udp, &owned).is_err());
                assert!(listener_inode("header\n0: truncated\n", v6, udp, &owned).is_err());
            }
        }
    }

    #[test]
    fn process_stat_handles_whitespace_and_parentheses_and_rejects_pid_aliases() {
        let stat = format!(
            "42 (dns (resolver) name) S {} 987 0",
            std::iter::repeat_n("0", 18).collect::<Vec<_>>().join(" ")
        );
        assert_eq!(starttime(&stat, 42).unwrap(), 987);
        assert!(starttime(&stat, 43).is_err());
        assert!(starttime("42 truncated", 42).is_err());
    }

    #[test]
    fn owned_udp_listener_cannot_hide_a_competing_connected_socket() {
        for v6 in [false, true] {
            let owned = BTreeSet::from([123]);
            let valid = row(v6, true, 123);
            let foreign = row(v6, true, 124).replace(" 07 ", " 01 ");
            let combined = valid + foreign.lines().nth(1).unwrap() + "\n";
            assert!(
                listener_inode(&combined, v6, true, &owned).is_err(),
                "connected foreign UDP socket was omitted from endpoint ownership evidence"
            );
        }
    }

    #[test]
    fn snapshots_bind_unit_process_namespace_cgroup_generation_and_every_socket() {
        let root = tempfile::tempdir().unwrap();
        let proc = root.path().join("proc");
        let cgroups = root.path().join("cgroups");
        let mut policy = super::super::tests::policy();
        let cg = "system.slice/system-direct.slice/direct-network-dns.service";
        policy.dns_cgroup = Some(cg.into());
        for path in [
            proc.join("42/fd"),
            proc.join("42/net"),
            proc.join("42/ns"),
            proc.join("self/ns"),
            cgroups.join(cg),
        ] {
            fs::create_dir_all(path).unwrap();
        }
        fs::write(proc.join("self/ns/net"), "namespace").unwrap();
        fs::hard_link(proc.join("self/ns/net"), proc.join("42/ns/net")).unwrap();
        fs::write(proc.join("42/cgroup"), format!("0::/{cg}\n")).unwrap();
        fs::write(
            proc.join("42/stat"),
            format!(
                "42 (dnsmasq) S {} 987 0",
                std::iter::repeat_n("0", 18).collect::<Vec<_>>().join(" ")
            ),
        )
        .unwrap();
        for (i, (name, v6, udp)) in [
            ("tcp", false, false),
            ("tcp6", true, false),
            ("udp", false, true),
            ("udp6", true, true),
        ]
        .into_iter()
        .enumerate()
        {
            let inode = 123 + i as u64;
            fs::write(proc.join("42/net").join(name), row(v6, udp, inode)).unwrap();
            symlink(
                format!("socket:[{inode}]"),
                proc.join("42/fd").join(i.to_string()),
            )
            .unwrap();
        }
        let enrolled =
            BTreeMap::from([(cg.to_owned(), fs::metadata(cgroups.join(cg)).unwrap().ino())]);
        let properties =
            format!("ActiveState=active\nSubState=running\nMainPID=42\nControlGroup=/{cg}\n");
        let valid = capture(&properties, &policy, &enrolled, &proc, &cgroups).unwrap();
        assert_eq!(valid.sockets, [123, 124, 125, 126]);
        for (v6, address) in [
            (
                true,
                IpAddr::V6(Ipv4Addr::new(127, 0, 0, 54).to_ipv6_mapped()),
            ),
            (true, IpAddr::V6(Ipv6Addr::UNSPECIFIED)),
            (false, IpAddr::V4(Ipv4Addr::UNSPECIFIED)),
        ] {
            for udp in [true, false] {
                let name = match (v6, udp) {
                    (true, true) => "udp6",
                    (true, false) => "tcp6",
                    (false, true) => "udp",
                    (false, false) => "tcp",
                };
                let path = proc.join("42/net").join(name);
                let original = fs::read_to_string(&path).unwrap();
                let foreign = row_at(address, udp, 999);
                fs::write(
                    &path,
                    original.clone() + foreign.lines().nth(1).unwrap() + "\n",
                )
                .unwrap();
                assert!(
                    capture(&properties, &policy, &enrolled, &proc, &cgroups).is_err(),
                    "competing alias or wildcard at the DNS port was ignored"
                );
                if udp {
                    fs::write(
                        &path,
                        original.clone()
                            + foreign.replace(" 07 ", " 01 ").lines().nth(1).unwrap()
                            + "\n",
                    )
                    .unwrap();
                    assert!(
                        capture(&properties, &policy, &enrolled, &proc, &cgroups).is_err(),
                        "connected wildcard or mapped alias was ignored"
                    );
                }
                fs::write(&path, original).unwrap();
            }
        }
        for name in ["tcp", "tcp6", "udp", "udp6"] {
            let path = proc.join("42/net").join(name);
            let before = fs::read(&path).unwrap();
            fs::write(&path, "header\n").unwrap();
            assert!(capture(&properties, &policy, &enrolled, &proc, &cgroups).is_err());
            fs::write(path, before).unwrap();
        }
        assert!(
            capture(
                &properties.replace("MainPID=42", "MainPID=43"),
                &policy,
                &enrolled,
                &proc,
                &cgroups
            )
            .is_err()
        );
        assert!(
            capture(
                &(properties.clone() + "MainPID=42\n"),
                &policy,
                &enrolled,
                &proc,
                &cgroups
            )
            .is_err()
        );
        assert!(
            capture(
                &properties.replace("ActiveState=active", "ActiveState=activating"),
                &policy,
                &enrolled,
                &proc,
                &cgroups
            )
            .is_err()
        );
        fs::remove_file(proc.join("42/fd/3")).unwrap();
        assert!(capture(&properties, &policy, &enrolled, &proc, &cgroups).is_err());
        symlink("socket:[126]", proc.join("42/fd/3")).unwrap();
        fs::rename(cgroups.join(cg), cgroups.join("old")).unwrap();
        fs::create_dir(cgroups.join(cg)).unwrap();
        assert!(capture(&properties, &policy, &enrolled, &proc, &cgroups).is_err());
        fs::remove_dir(cgroups.join(cg)).unwrap();
        fs::rename(cgroups.join("old"), cgroups.join(cg)).unwrap();
        fs::remove_file(proc.join("42/ns/net")).unwrap();
        fs::write(proc.join("42/ns/net"), "foreign namespace").unwrap();
        assert!(capture(&properties, &policy, &enrolled, &proc, &cgroups).is_err());
        policy.dns_cgroup = None;
        assert!(capture(&properties, &policy, &enrolled, &proc, &cgroups).is_err());
    }
}
