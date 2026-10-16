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
        {"ifindex": 6, "ifname": "vlan-container", "link_type": "ether", "link": "br-container", "linkinfo": {"info_kind": "vlan"}},
        {"ifindex": 7, "ifname": "vlan-physical", "link_type": "ether", "link": "enp1s0", "linkinfo": {"info_kind": "vlan"}}
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
fn vlan_parent_names_trace_nested_uplinks_but_foreign_indices_and_cycles_do_not() {
    let links = serde_json::json!([
        {"ifindex": 1, "ifname": "enp1s0", "link_type": "ether"},
        {"ifindex": 2, "ifname": "container0", "link_type": "ether", "linkinfo": {"info_kind": "veth"}},
        {"ifindex": 3, "ifname": "vlan-nested", "link_type": "ether", "link": "vlan-physical", "linkinfo": {"info_kind": "vlan"}},
        {"ifindex": 4, "ifname": "vlan-physical", "link_type": "ether", "link": "enp1s0", "linkinfo": {"info_kind": "vlan"}},
        {"ifindex": 5, "ifname": "vlan-container", "link_type": "ether", "link": "container0", "linkinfo": {"info_kind": "vlan"}},
        {"ifindex": 6, "ifname": "vlan-foreign", "link_type": "ether", "link_index": 1, "link_netnsid": 0, "linkinfo": {"info_kind": "vlan"}},
        {"ifindex": 7, "ifname": "vlan-cycle-a", "link_type": "ether", "link": "vlan-cycle-b", "linkinfo": {"info_kind": "vlan"}},
        {"ifindex": 8, "ifname": "vlan-cycle-b", "link_type": "ether", "link": "vlan-cycle-a", "linkinfo": {"info_kind": "vlan"}}
    ]);
    let routes = serde_json::json!([
        {"dst": "10.0.1.0/24", "dev": "vlan-nested"},
        {"dst": "10.0.2.0/24", "dev": "vlan-physical"},
        {"dst": "10.0.3.0/24", "dev": "vlan-container"},
        {"dst": "10.0.4.0/24", "dev": "vlan-foreign"},
        {"dst": "10.0.5.0/24", "dev": "vlan-cycle-a"},
        {"dst": "10.0.6.0/24", "dev": "vlan-cycle-b"}
    ]);
    let selected = select_routes(&policy(), &links, &routes).unwrap();
    assert_eq!(
        selected
            .iter()
            .map(|r| r.device.as_str())
            .collect::<Vec<_>>(),
        ["vlan-nested", "vlan-physical"]
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

#[test]
fn rejects_oversized_link_snapshots_before_selecting_routes() {
    let link = serde_json::json!({"ifname":"wg-home","linkinfo":{"info_kind":"wireguard"}});
    let routes = serde_json::json!([{"dst":"default","dev":"wg-home"}]);
    let at_limit = serde_json::Value::Array(vec![link.clone(); 4096]);
    assert_eq!(
        select_routes(&policy(), &at_limit, &routes).unwrap().len(),
        1
    );
    let oversized = serde_json::Value::Array(vec![link; 4097]);
    assert!(select_routes(&policy(), &oversized, &routes).is_err());
}

#[test]
fn rejects_non_array_link_snapshots_instead_of_accepting_empty_topology() {
    assert!(select_routes(&policy(), &serde_json::json!({}), &serde_json::json!([])).is_err());
}

#[test]
fn multipath_kernel_status_flags_do_not_discard_the_route_or_authored_onlink() {
    let links = serde_json::json!([
        {"ifname":"enp1s0","link_type":"ether"},
        {"ifname":"enp2s0","link_type":"ether"}
    ]);
    for status in [
        "dead",
        "linkdown",
        "unresolved",
        "offload",
        "trap",
        "notify",
        "rt_offload",
        "rt_trap",
        "rt_offload_failed",
    ] {
        let routes = serde_json::json!([{"dst":"default","nexthops":[
            {"dev":"enp1s0","gateway":"192.0.2.1","weight":2,"flags":[status,"onlink"]},
            {"dev":"enp2s0","gateway":"198.51.100.1","weight":3,"flags":[]}
        ]}]);
        let selected = select_routes(&policy(), &links, &routes).unwrap();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].nexthops.len(), 2);
        assert_eq!(selected[0].nexthops[0].weight, 2);
        assert!(selected[0].nexthops[0].onlink);
        assert_eq!(selected[0].nexthops[1].weight, 3);
        assert!(!selected[0].nexthops[1].onlink);
    }
}

#[test]
fn multipath_encapsulation_rejects_the_snapshot_instead_of_rewriting_forwarding() {
    let links = serde_json::json!([
        {"ifname":"enp1s0","link_type":"ether"},
        {"ifname":"enp2s0","link_type":"ether"}
    ]);
    for encapsulation in [
        serde_json::json!({"encap_type":"mpls","dst":"100"}),
        serde_json::json!({"encap_type":"seg6","mode":"encap","segs":["2001:db8::1"]}),
        serde_json::Value::Null,
    ] {
        for index in 0..2 {
            let mut routes = serde_json::json!([{"dst":"default","nexthops":[
                {"dev":"enp1s0","gateway":"192.0.2.1","weight":2,"flags":["onlink"]},
                {"dev":"enp2s0","gateway":"198.51.100.1","weight":3}
            ]}]);
            routes[0]["nexthops"][index]["encap"] = encapsulation.clone();
            let error = select_routes(&policy(), &links, &routes).unwrap_err();
            assert!(error.to_string().contains("encapsulation"));
        }
    }
}

#[test]
fn destination_rewrites_reject_eligible_routes_and_each_multipath_hop() {
    let links = serde_json::json!([{"ifname":"enp1s0","link_type":"ether"}]);
    for rewrite in [
        serde_json::json!("192.0.2.10"),
        serde_json::json!("2001:db8::10"),
        serde_json::Value::Null,
        serde_json::json!({"malformed":true}),
    ] {
        for mut route in [
            serde_json::json!({"dst":"default","dev":"enp1s0"}),
            serde_json::json!({"dst":"default","nexthops":[
                {"dev":"enp1s0","gateway":"192.0.2.1","weight":2},
                {"dev":"enp1s0","gateway":"192.0.2.2","weight":3}
            ]}),
        ] {
            route["to"] = rewrite.clone();
            let error = select_routes(&policy(), &links, &serde_json::json!([route])).unwrap_err();
            assert!(error.to_string().contains("destination rewrite"));
        }
        for index in 0..2 {
            let mut route = serde_json::json!({"dst":"default","nexthops":[
                {"dev":"enp1s0","gateway":"192.0.2.1","weight":2},
                {"dev":"enp1s0","gateway":"192.0.2.2","weight":3}
            ]});
            route["nexthops"][index]["to"] = rewrite.clone();
            let error = select_routes(&policy(), &links, &serde_json::json!([route])).unwrap_err();
            assert!(error.to_string().contains("destination rewrite"));
        }
    }
    for route in [
        serde_json::json!({"dst":"default","dev":"wg-proton","to":"192.0.2.10"}),
        serde_json::json!({"dst":"default","to":"192.0.2.10","nexthops":[
            {"dev":"enp1s0","to":"192.0.2.11"},
            {"dev":"wg-proton"}
        ]}),
        serde_json::json!({"dst":"default","nexthops":[
            {"dev":"wg-proton","to":"192.0.2.10"},
            {"dev":"enp1s0"}
        ]}),
    ] {
        assert!(
            select_routes(&policy(), &links, &serde_json::json!([route]))
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn single_path_flags_reject_authored_pervasive_and_malformed_values() {
    let links = serde_json::json!([{"ifname":"enp1s0","link_type":"ether"}]);
    for flags in [
        serde_json::json!(["pervasive"]),
        serde_json::json!(["onlink", "pervasive"]),
        serde_json::json!(["future-authored-flag"]),
        serde_json::json!([null]),
        serde_json::json!("onlink"),
        serde_json::Value::Null,
    ] {
        let route = serde_json::json!({"dst":"default","dev":"enp1s0","flags":flags});
        let error = select_routes(&policy(), &links, &serde_json::json!([route])).unwrap_err();
        assert!(error.to_string().contains("route flags"));
        let omitted = serde_json::json!([
            {"dst":"default","dev":"wg-proton","flags":flags},
            {"dst":"default","flags":flags,"nexthops":[
                {"dev":"enp1s0"}, {"dev":"wg-proton"}
            ]}
        ]);
        assert!(
            select_routes(&policy(), &links, &omitted)
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn single_path_kernel_status_flags_preserve_authored_onlink() {
    let links = serde_json::json!([{"ifname":"enp1s0","link_type":"ether"}]);
    for status in [
        "dead",
        "linkdown",
        "unresolved",
        "offload",
        "trap",
        "notify",
        "rt_offload",
        "rt_trap",
        "rt_offload_failed",
    ] {
        for onlink in [false, true] {
            let flags = if onlink {
                vec![status, "onlink"]
            } else {
                vec![status]
            };
            let routes = serde_json::json!([
                {"dst":"default","dev":"enp1s0","gateway":"192.0.2.1","flags":flags}
            ]);
            let selected = select_routes(&policy(), &links, &routes).unwrap();
            assert_eq!(selected.len(), 1);
            assert_eq!(selected[0].onlink, onlink);
        }
    }
}

#[test]
fn route_encapsulation_rejects_eligible_single_and_multipath_snapshots() {
    let links = serde_json::json!([{"ifname":"enp1s0","link_type":"ether"}]);
    for mut route in [
        serde_json::json!({"dst":"default","dev":"enp1s0","gateway":"192.0.2.1"}),
        serde_json::json!({"dst":"default","nexthops":[
            {"dev":"enp1s0","gateway":"192.0.2.1","weight":2}
        ]}),
    ] {
        route["encap"] = serde_json::json!({"encap_type":"mpls","dst":"100"});
        let error = select_routes(&policy(), &links, &serde_json::json!([route])).unwrap_err();
        assert!(error.to_string().contains("encapsulation"));
    }
}

#[test]
fn route_metrics_reject_eligible_snapshots_instead_of_changing_path_semantics() {
    let links = serde_json::json!([{"ifname":"enp1s0","link_type":"ether"}]);
    for metrics in [
        serde_json::json!({"mtu":1400,"advmss":1200}),
        serde_json::json!([{"mtu":1400,"lock":true}]),
        serde_json::json!({"congctl":"bbr","rto_min":200}),
        serde_json::json!({}),
        serde_json::Value::Null,
        serde_json::json!("malformed"),
    ] {
        for mut route in [
            serde_json::json!({"dst":"default","dev":"enp1s0","gateway":"192.0.2.1"}),
            serde_json::json!({"dst":"default","nexthops":[
                {"dev":"enp1s0","gateway":"192.0.2.1","weight":2}
            ]}),
        ] {
            route["metrics"] = metrics.clone();
            let error = select_routes(&policy(), &links, &serde_json::json!([route])).unwrap_err();
            assert!(error.to_string().contains("route metrics"));
        }
    }
    // Metrics on producer-owned VPN paths do not make them eligible.
    for route in [
        serde_json::json!({"dst":"default","dev":"wg-proton","metrics":{"mtu":1400}}),
        serde_json::json!({"dst":"default","metrics":{"mtu":1400},"nexthops":[
            {"dev":"enp1s0","gateway":"192.0.2.1"},
            {"dev":"wg-proton","gateway":"198.51.100.1"}
        ]}),
    ] {
        assert!(
            select_routes(&policy(), &links, &serde_json::json!([route]))
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn ttl_propagation_rejects_eligible_routes_without_blocking_omitted_paths() {
    let links = serde_json::json!([{"ifname":"enp1s0","link_type":"ether"}]);
    for propagation in [
        serde_json::json!(true),
        serde_json::json!(false),
        serde_json::Value::Null,
        serde_json::json!("enabled"),
        serde_json::json!({"malformed":true}),
    ] {
        for mut route in [
            serde_json::json!({"dst":"default","dev":"enp1s0","gateway":"192.0.2.1"}),
            serde_json::json!({"dst":"default","nexthops":[
                {"dev":"enp1s0","gateway":"192.0.2.1","weight":2},
                {"dev":"enp1s0","gateway":"192.0.2.2","weight":3}
            ]}),
            serde_json::json!({"dst":"2001:db8::/64","dev":"enp1s0","gateway":"fe80::1"}),
            serde_json::json!({"dst":"2001:db8::/64","nexthops":[
                {"dev":"enp1s0","gateway":"fe80::1","weight":2},
                {"dev":"enp1s0","gateway":"fe80::2","weight":3}
            ]}),
        ] {
            // iproute2's JSON printer uses this misspelled key for RTA_TTL_PROPAGATE.
            route["ttl-propogate"] = propagation.clone();
            let error = select_routes(&policy(), &links, &serde_json::json!([route])).unwrap_err();
            assert!(error.to_string().contains("TTL propagation"));
        }
        for mut route in [
            serde_json::json!({"dst":"default","dev":"wg-proton"}),
            serde_json::json!({"dst":"default","dev":"veth-container"}),
            serde_json::json!({"dst":"default","nexthops":[
                {"dev":"enp1s0"}, {"dev":"wg-proton"}
            ]}),
            serde_json::json!({"dst":"default","nexthops":[
                {"dev":"wg-proton"}, {"dev":"enp1s0"}
            ]}),
        ] {
            route["ttl-propogate"] = propagation.clone();
            assert!(
                select_routes(&policy(), &links, &serde_json::json!([route]))
                    .unwrap()
                    .is_empty()
            );
        }
    }
}

#[test]
fn route_lifetimes_reject_eligible_snapshots_instead_of_becoming_permanent() {
    let links = serde_json::json!([{"ifname":"enp1s0","link_type":"ether"}]);
    for expires in [
        serde_json::json!(120),
        serde_json::json!(0),
        serde_json::json!(1.25),
        serde_json::json!("120sec"),
        serde_json::Value::Null,
    ] {
        for mut route in [
            serde_json::json!({"dst":"2001:db8:2::/64","dev":"enp1s0","gateway":"fe80::1"}),
            serde_json::json!({"dst":"2001:db8:2::/64","nexthops":[
                {"dev":"enp1s0","gateway":"fe80::1","weight":2}
            ]}),
        ] {
            route["expires"] = expires.clone();
            let error = select_routes(&policy(), &links, &serde_json::json!([route])).unwrap_err();
            assert!(error.to_string().contains("route lifetime"));
        }
    }
    // Expiration on VPN-owned or mixed paths must not enroll those routes.
    for route in [
        serde_json::json!({"dst":"default","dev":"wg-proton","expires":120}),
        serde_json::json!({"dst":"default","expires":120,"nexthops":[
            {"dev":"enp1s0","gateway":"fe80::1"},
            {"dev":"wg-proton","gateway":"fe80::2"}
        ]}),
    ] {
        assert!(
            select_routes(&policy(), &links, &serde_json::json!([route]))
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn source_selectors_cannot_become_source_unconditional_or_connected_routes() {
    let links = serde_json::json!([{"ifname":"enp1s0","link_type":"ether"}]);
    for key in ["from", "src"] {
        for source in [
            serde_json::json!("2001:db8:1::/64"),
            serde_json::json!("0/64"),
            serde_json::Value::Null,
        ] {
            for mut route in [
                serde_json::json!({"dst":"2001:db8:2::/64","dev":"enp1s0"}),
                serde_json::json!({"dst":"2001:db8:2::/64","nexthops":[
                    {"dev":"enp1s0","gateway":"fe80::1","weight":2}
                ]}),
            ] {
                route[key] = source.clone();
                let error =
                    select_routes(&policy(), &links, &serde_json::json!([route])).unwrap_err();
                assert!(error.to_string().contains("source selector"));
            }
        }
    }
    let routes = serde_json::json!([{"dst":"default","from":"2001:db8:1::/64","nexthops":[
        {"dev":"enp1s0","gateway":"fe80::1"}, {"dev":"wg-proton","gateway":"fe80::2"}
    ]}]);
    assert!(
        select_routes(&policy(), &links, &routes)
            .unwrap()
            .is_empty()
    );
    // Preferred source is a different attribute, not a packet-source selector.
    let routes = serde_json::json!([{"dst":"default","prefsrc":"2001:db8:1::1","nexthops":[
        {"dev":"enp1s0","gateway":"fe80::1","weight":2}
    ]}]);
    assert_eq!(
        select_routes(&policy(), &links, &routes).unwrap()[0]
            .source
            .as_deref(),
        Some("2001:db8:1::1")
    );
}

#[test]
fn referenced_nexthops_cannot_be_rewritten_as_ordinary_inline_ecmp() {
    let links = serde_json::json!([{"ifname":"enp1s0","link_type":"ether"}]);
    for id in [
        serde_json::json!(112),
        serde_json::json!("112"),
        serde_json::Value::Null,
    ] {
        for mut route in [
            serde_json::json!({"dst":"default","dev":"enp1s0","gateway":"192.0.2.1"}),
            serde_json::json!({"dst":"default","nexthops":[
                {"dev":"enp1s0","gateway":"192.0.2.1","weight":2}
            ]}),
        ] {
            route["nhid"] = id.clone();
            let error = select_routes(&policy(), &links, &serde_json::json!([route])).unwrap_err();
            assert!(error.to_string().contains("referenced nexthop"));
        }
    }
    for route in [
        serde_json::json!({"dst":"default","nhid":112}),
        serde_json::json!({"dst":"default","nhid":112,"nexthops":[
            {"dev":"enp1s0","gateway":"192.0.2.1"}, {"dev":"wg-proton","gateway":"198.51.100.1"}
        ]}),
    ] {
        assert!(
            select_routes(&policy(), &links, &serde_json::json!([route]))
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn nondefault_ecmp_scope_is_rejected_instead_of_becoming_universe_scoped() {
    let links = serde_json::json!([{"ifname":"enp1s0","link_type":"ether"}]);
    let mut route = serde_json::json!({"dst":"192.0.2.0/24","nexthops":[
        {"dev":"enp1s0","weight":2}, {"dev":"enp1s0","weight":3}
    ]});
    let expected = select_routes(&policy(), &links, &serde_json::json!([route])).unwrap();
    route["scope"] = serde_json::json!("global");
    assert_eq!(
        select_routes(&policy(), &links, &serde_json::json!([route])).unwrap(),
        expected
    );
    for scope in [
        serde_json::json!("link"),
        serde_json::json!("host"),
        serde_json::json!(253),
        serde_json::Value::Null,
    ] {
        route["scope"] = scope;
        let error = select_routes(&policy(), &links, &serde_json::json!([route])).unwrap_err();
        assert!(error.to_string().contains("scope"));
    }
    route["nexthops"][1]["dev"] = serde_json::json!("wg-proton");
    assert!(
        select_routes(&policy(), &links, &serde_json::json!([route]))
            .unwrap()
            .is_empty()
    );
    let connected = serde_json::json!([{"dst":"192.0.2.0/24","dev":"enp1s0","scope":"link"}]);
    assert!(select_routes(&policy(), &links, &connected).unwrap()[0].local);
}

#[test]
fn canonical_scope_is_retained_in_snapshot_identity_and_malformed_scopes_reject() {
    let links = serde_json::json!([{"ifname":"enp1s0","link_type":"ether"}]);
    let mut route = serde_json::json!({"dst":"192.0.2.0/24","dev":"enp1s0"});
    let global = select_routes(&policy(), &links, &serde_json::json!([route])).unwrap();
    assert_eq!(serde_json::to_value(&global[0]).unwrap()["scope"], "global");
    for scope in ["link", "host"] {
        route["scope"] = serde_json::json!(scope);
        let selected = select_routes(&policy(), &links, &serde_json::json!([route])).unwrap();
        assert_ne!(selected, global);
        assert_eq!(serde_json::to_value(&selected[0]).unwrap()["scope"], scope);
    }
    for invalid in [
        serde_json::json!("site"),
        serde_json::json!(253),
        serde_json::Value::Null,
        serde_json::json!({}),
    ] {
        route["scope"] = invalid;
        assert!(select_routes(&policy(), &links, &serde_json::json!([route])).is_err());
    }
}

#[test]
fn tos_selectors_reject_eligible_routes_instead_of_widening_their_match() {
    let links = serde_json::json!([{"ifname":"enp1s0","link_type":"ether"}]);
    for tos in [
        serde_json::json!("0x10"),
        serde_json::json!(16),
        serde_json::json!(0),
        serde_json::Value::Null,
        serde_json::json!({}),
    ] {
        for mut route in [
            serde_json::json!({"dst":"192.0.2.0/24","dev":"enp1s0","gateway":"192.0.2.1"}),
            serde_json::json!({"dst":"192.0.2.0/24","nexthops":[
                {"dev":"enp1s0","gateway":"192.0.2.1","weight":2}
            ]}),
        ] {
            route["tos"] = tos.clone();
            let error = select_routes(&policy(), &links, &serde_json::json!([route])).unwrap_err();
            assert!(error.to_string().contains("TOS"));
        }
    }
    let routes = serde_json::json!([{"dst":"default","tos":"0x10","nexthops":[
        {"dev":"enp1s0","gateway":"192.0.2.1"},
        {"dev":"wg-proton","gateway":"198.51.100.1"}
    ]}]);
    assert!(
        select_routes(&policy(), &links, &routes)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn nondefault_or_malformed_ipv6_preferences_reject_without_changing_the_uplink() {
    let links = serde_json::json!([{"ifname":"enp1s0","link_type":"ether"}]);
    for preference in [
        serde_json::json!("high"),
        serde_json::json!("low"),
        serde_json::json!("MEDIUM"),
        serde_json::json!(0),
        serde_json::Value::Null,
        serde_json::json!({}),
    ] {
        for mut route in [
            serde_json::json!({"dst":"default","dev":"enp1s0","gateway":"fe80::1"}),
            serde_json::json!({"dst":"default","nexthops":[
                {"dev":"enp1s0","gateway":"fe80::1","weight":2}
            ]}),
        ] {
            route["pref"] = preference.clone();
            let error = select_routes(&policy(), &links, &serde_json::json!([route])).unwrap_err();
            assert!(error.to_string().contains("preference"));
        }
    }
    // iproute2 emits the default medium preference on ordinary IPv6 paths.
    for mut route in [
        serde_json::json!({"dst":"default","dev":"enp1s0","gateway":"fe80::1"}),
        serde_json::json!({"dst":"default","nexthops":[
            {"dev":"enp1s0","gateway":"fe80::1","weight":2}
        ]}),
    ] {
        let expected = select_routes(&policy(), &links, &serde_json::json!([route])).unwrap();
        route["pref"] = serde_json::json!("medium");
        assert_eq!(
            select_routes(&policy(), &links, &serde_json::json!([route])).unwrap(),
            expected
        );
    }
    let routes = serde_json::json!([{"dst":"default","pref":"high","nexthops":[
        {"dev":"enp1s0","gateway":"fe80::1"},
        {"dev":"wg-proton","gateway":"fe80::2"}
    ]}]);
    assert!(
        select_routes(&policy(), &links, &routes)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn route_and_nexthop_realms_reject_eligible_snapshots_without_losing_classification() {
    let links = serde_json::json!([{"ifname":"enp1s0","link_type":"ether"}]);
    for flow in [
        serde_json::json!({"to":"1"}),
        serde_json::json!({"from":"1","to":"2"}),
        serde_json::json!({}),
        serde_json::Value::Null,
        serde_json::json!("malformed"),
    ] {
        for location in 0..4 {
            let mut route = if location == 0 {
                serde_json::json!({"dst":"default","dev":"enp1s0","gateway":"192.0.2.1"})
            } else {
                serde_json::json!({"dst":"default","nexthops":[
                    {"dev":"enp1s0","gateway":"192.0.2.1","weight":2},
                    {"dev":"enp1s0","gateway":"192.0.2.2","weight":3}
                ]})
            };
            if location < 2 {
                route["flow"] = flow.clone();
            } else {
                route["nexthops"][location - 2]["flow"] = flow.clone();
            }
            let error = select_routes(&policy(), &links, &serde_json::json!([route])).unwrap_err();
            assert!(error.to_string().contains("realms"));
        }
    }
    for route in [
        serde_json::json!({"dst":"default","dev":"wg-proton","flow":{"to":"1"}}),
        serde_json::json!({"dst":"default","flow":{"to":"1"},"nexthops":[
            {"dev":"enp1s0","gateway":"192.0.2.1","flow":{"to":"1"}},
            {"dev":"wg-proton","gateway":"198.51.100.1","flow":{"to":"1"}}
        ]}),
    ] {
        assert!(
            select_routes(&policy(), &links, &serde_json::json!([route]))
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn oversized_producer_owned_ecmp_groups_are_omitted_before_direct_replay_limits() {
    let links = serde_json::json!([
        {"ifname":"enp1s0","link_type":"ether"},
        {"ifname":"wg-proton","link_type":"none","linkinfo":{"info_kind":"wireguard"}},
        {"ifname":"docker0","link_type":"ether","linkinfo":{"info_kind":"bridge"}}
    ]);
    let direct = serde_json::json!({"dst":"default","dev":"enp1s0","gateway":"192.0.2.1"});
    let expected = select_routes(&policy(), &links, &serde_json::json!([direct])).unwrap();
    for count in [61, 64, 128, 256] {
        for ineligible in ["wg-proton", "docker0"] {
            for direct_members in [0, count - 1] {
                let mut hops = (0..count)
                    .map(|index| {
                        serde_json::json!({
                            "dev":if index < direct_members {"enp1s0"} else {ineligible},
                            "gateway":"192.0.2.1", "weight":1
                        })
                    })
                    .collect::<Vec<_>>();
                // Put the only ineligible member last, then first, proving the
                // complete group is checked before imposing replay capacity.
                for _ in 0..2 {
                    let routes =
                        serde_json::json!([direct,{"dst":"198.51.100.0/24","nexthops":hops}]);
                    assert_eq!(select_routes(&policy(), &links, &routes).unwrap(), expected);
                    hops.reverse();
                }
            }
        }
    }
}

#[test]
fn ecmp_replay_limit_rejects_oversized_groups_without_truncating_supported_groups() {
    let links = serde_json::json!([{"ifname":"enp1s0","link_type":"ether"}]);
    for invalid in [
        serde_json::json!([]),
        serde_json::Value::Null,
        serde_json::json!({"dev":"enp1s0"}),
        serde_json::json!("malformed"),
    ] {
        let routes = serde_json::json!([{"dst":"default", "nexthops":invalid}]);
        assert!(select_routes(&policy(), &links, &routes).is_err());
    }
    for gateway in ["192.0.2.1", "2001:db8::1"] {
        for count in [60, 61, 128, 160, 256] {
            let hops = (0..count)
                .map(|index| {
                    serde_json::json!({
                        "dev":"enp1s0", "gateway":gateway, "weight":index % 256 + 1,
                        "flags":["onlink"]
                    })
                })
                .collect::<Vec<_>>();
            let routes = serde_json::json!([{"dst":"default", "nexthops":hops}]);
            let result = select_routes(&policy(), &links, &routes);
            if count == 60 {
                let selected = result.unwrap();
                assert_eq!(selected[0].nexthops.len(), count);
                assert_eq!(selected[0].nexthops[count - 1].weight, count as u64);
                assert!(selected[0].nexthops.iter().all(|hop| hop.onlink));
            } else {
                assert!(result.is_err(), "accepted {count} nexthops with {gateway}");
            }
        }
    }
}

#[test]
fn cross_family_via_rejects_eligible_routes_instead_of_becoming_connected() {
    let links = serde_json::json!([{"ifname":"enp1s0","link_type":"ether"}]);
    for via in [
        serde_json::json!({"family":"inet6","host":"fe80::1"}),
        serde_json::json!({"family":"inet","host":"192.0.2.1"}),
        serde_json::Value::Null,
    ] {
        for mut route in [
            serde_json::json!({"dst":"192.0.2.0/24","dev":"enp1s0"}),
            serde_json::json!({"dst":"192.0.2.0/24","nexthops":[
                {"dev":"enp1s0","weight":2,"flags":["onlink"]}
            ]}),
        ] {
            if route.get("nexthops").is_some() {
                route["nexthops"][0]["via"] = via.clone();
            } else {
                route["via"] = via.clone();
            }
            let error = select_routes(&policy(), &links, &serde_json::json!([route])).unwrap_err();
            assert!(error.to_string().contains("cross-family"));
        }
    }
    // A producer-owned VPN path remains omitted as a whole, including its via.
    let routes = serde_json::json!([{"dst":"default","nexthops":[
        {"dev":"enp1s0","gateway":"192.0.2.1"},
        {"dev":"wg-proton","via":{"family":"inet6","host":"fe80::1"}}
    ]}]);
    assert!(
        select_routes(&policy(), &links, &routes)
            .unwrap()
            .is_empty()
    );
}
