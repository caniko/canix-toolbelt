#![cfg(all(target_os = "linux", feature = "direct-network"))]

use canix_toolbelt::direct_network::{Policy, select_routes};

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
