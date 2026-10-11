#![cfg(all(target_os = "linux", feature = "direct-network"))]

use canix_toolbelt::direct_network::{Policy, ready, select_routes};
use std::io::{Read, Write};
use std::os::unix::net::UnixListener;
use std::thread;
use std::time::Duration;

#[test]
fn readiness_waits_for_socket_creation_and_pending_initial_reconciliation() {
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("ready.sock");
    let server_socket = socket.clone();
    let server = thread::spawn(move || {
        thread::sleep(Duration::from_millis(150));
        let listener = UnixListener::bind(server_socket).unwrap();
        for response in [b"not-ready\n".as_slice(), b"ready\n".as_slice()] {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 6];
            stream.read_exact(&mut request).unwrap();
            assert_eq!(&request, b"ready\n");
            stream.write_all(response).unwrap();
        }
    });
    ready(&socket).unwrap();
    server.join().unwrap();
}

fn policy() -> Policy {
    serde_json::from_value(serde_json::json!({
        "socket": "/run/direct-network/ready.sock",
        "serversFile": "/run/direct-network/servers",
        "ip": "/usr/bin/ip",
        "nft": "/usr/bin/nft",
        "nmcli": "/usr/bin/nmcli",
        "systemctl": "/usr/bin/systemctl",
        "cgroupPaths": ["system.slice/system-direct.slice"],
        "allowedUsers": [1000],
        "directInterfaces": ["wg-home"],
        "dnsServers": [],
        "dnsZones": {"vpn.example.test": "10.123.0.1"},
        "networkManagerDns": true
    }))
    .unwrap()
}

#[test]
fn rejects_vpn_and_kill_switch_routes_but_keeps_fleet_and_current_uplink() {
    let links = serde_json::json!([
        {"ifname": "enp1s0", "link_type": "ether"},
        {"ifname": "wg-home", "link_type": "none", "linkinfo": {"info_kind": "wireguard"}},
        {"ifname": "proton0", "link_type": "none", "linkinfo": {"info_kind": "wireguard"}},
        {"ifname": "pvpnksintrf0", "link_type": "ether", "linkinfo": {"info_kind": "dummy"}},
        {"ifname": "container0", "link_type": "ether", "linkinfo": {"info_kind": "veth"}}
    ]);
    let routes = serde_json::json!([
        {"dst": "default", "gateway": "192.0.2.1", "dev": "enp1s0", "metric": 100},
        {"dst": "192.0.2.0/24", "dev": "enp1s0", "scope": "link", "prefsrc": "192.0.2.2"},
        {"dst": "10.123.0.0/24", "dev": "wg-home"},
        {"dst": "default", "dev": "pvpnksintrf0", "metric": 1},
        {"dst": "0.0.0.0/1", "dev": "proton0"},
        {"dst": "198.18.0.0/16", "dev": "container0"},
        {"type": "blackhole", "dst": "default"}
    ]);
    let selected = select_routes(&policy(), &links, &routes).unwrap();
    assert_eq!(selected.len(), 3);
    assert!(selected.iter().any(|r| r.destination == "10.123.0.0/24"));
    assert!(
        selected
            .iter()
            .any(|r| r.gateway.as_deref() == Some("192.0.2.1"))
    );
    assert!(selected.iter().all(|r| !r.device.starts_with("pvpn")));
}

#[test]
fn preserves_ipv6_gateway_scope_and_existing_source_address() {
    let selected = select_routes(
        &policy(),
        &serde_json::json!([{"ifname": "wlan0", "link_type": "ether"}]),
        &serde_json::json!([
            {"dst": "default", "gateway": "fe80::1", "dev": "wlan0", "metric": 600},
            {"dst": "2001:db8::/64", "dev": "wlan0", "prefsrc": "2001:db8::2"}
        ]),
    )
    .unwrap();
    assert!(
        selected
            .iter()
            .any(|r| r.gateway.as_deref() == Some("fe80::1"))
    );
    assert!(
        selected
            .iter()
            .any(|r| r.source.as_deref() == Some("2001:db8::2"))
    );
}

#[test]
fn malformed_runtime_tokens_cannot_become_ip_or_nft_commands() {
    let links = serde_json::json!([{"ifname": "eth0", "link_type": "ether"}]);
    assert!(
        select_routes(
            &policy(),
            &links,
            &serde_json::json!([
                {"dst": "default\nflush ruleset", "dev": "eth0"}
            ])
        )
        .is_err()
    );
    let mut p = policy();
    p.cgroup_paths.push("../user.slice".into());
    assert!(p.validate().is_err());
    let mut p = policy();
    p.dns_zones
        .insert("vpn.test\nserver=evil".into(), "10.123.0.1".into());
    assert!(p.validate().is_err());
}

#[test]
fn noncanonical_cgroup_separators_cannot_silently_change_socket_depth() {
    for value in [
        "system.slice//worker.slice",
        "system.slice/",
        "system.slice/./worker.slice",
    ] {
        let mut p = policy();
        p.cgroup_paths = vec![value.into()];
        assert!(
            p.validate().is_err(),
            "accepted noncanonical enrollment path {value}"
        );
        let mut p = policy();
        p.dns_cgroup = Some(value.into());
        assert!(
            p.validate().is_err(),
            "accepted noncanonical resolver path {value}"
        );
    }
    let mut p = policy();
    p.cgroup_paths = vec!["system.slice/worker.slice".into()];
    p.dns_cgroup = Some("system.slice/direct-network-dns.service".into());
    p.validate().unwrap();
}

#[test]
fn container_only_bridges_are_excluded_but_physical_and_explicit_bridges_remain_direct() {
    let links = serde_json::json!([
        {"ifindex": 1, "ifname": "enp1s0", "link_type": "ether", "master": "br-uplink"},
        {"ifindex": 2, "ifname": "br-uplink", "link_type": "ether", "linkinfo": {"info_kind": "bridge"}},
        {"ifindex": 3, "ifname": "br-container", "link_type": "ether", "linkinfo": {"info_kind": "bridge"}},
        {"ifindex": 4, "ifname": "veth0", "link_type": "ether", "master": "br-container", "linkinfo": {"info_kind": "veth"}},
        {"ifindex": 5, "ifname": "br-explicit", "link_type": "ether", "linkinfo": {"info_kind": "bridge"}},
        {"ifindex": 6, "ifname": "vlan-container", "link_type": "ether", "link_index": 3, "linkinfo": {"info_kind": "vlan"}},
        {"ifindex": 7, "ifname": "vlan-physical", "link_type": "ether", "link_index": 1, "linkinfo": {"info_kind": "vlan"}}
    ]);
    let routes = serde_json::json!([
        {"dst": "192.0.2.0/24", "dev": "br-uplink"},
        {"dst": "198.51.100.0/24", "dev": "br-container"},
        {"dst": "203.0.113.0/24", "dev": "br-explicit"},
        {"dst": "10.0.1.0/24", "dev": "vlan-container"},
        {"dst": "10.0.2.0/24", "dev": "vlan-physical"}
    ]);
    let mut p = policy();
    p.direct_interfaces.push("br-explicit".into());
    let selected = select_routes(&p, &links, &routes).unwrap();
    assert_eq!(
        selected
            .iter()
            .map(|r| r.device.as_str())
            .collect::<Vec<_>>(),
        ["br-uplink", "br-explicit", "vlan-physical"]
    );
}

#[test]
fn multipath_routes_retain_every_eligible_weighted_nexthop_and_reject_mixed_vpn_routes() {
    let links = serde_json::json!([
        {"ifname": "enp1s0", "link_type": "ether"},
        {"ifname": "enp2s0", "link_type": "ether"},
        {"ifname": "wg-proton", "link_type": "none", "linkinfo": {"info_kind": "wireguard"}}
    ]);
    let routes = serde_json::json!([
        {"dst": "default", "metric": 42, "nexthops": [
            {"dev": "enp1s0", "gateway": "192.0.2.1", "weight": 2, "flags": ["onlink"]},
            {"dev": "enp2s0", "gateway": "198.51.100.1", "weight": 3}
        ]},
        {"dst": "203.0.113.0/24", "nexthops": [
            {"dev": "enp1s0", "gateway": "192.0.2.1", "weight": 1},
            {"dev": "wg-proton", "gateway": "172.16.0.1", "weight": 1}
        ]}
    ]);
    let selected = select_routes(&policy(), &links, &routes).unwrap();
    assert_eq!(
        selected.len(),
        1,
        "eligible multipath default was omitted or mixed VPN route admitted"
    );
    let selected = serde_json::to_value(&selected[0]).unwrap();
    assert_eq!(selected["nexthops"][0]["device"], "enp1s0");
    assert_eq!(selected["nexthops"][0]["gateway"], "192.0.2.1");
    assert_eq!(selected["nexthops"][0]["weight"], 2);
    assert_eq!(selected["nexthops"][0]["onlink"], true);
    assert_eq!(selected["nexthops"][1]["device"], "enp2s0");
    assert_eq!(selected["nexthops"][1]["weight"], 3);
    assert_eq!(selected["metric"], 42);
}
