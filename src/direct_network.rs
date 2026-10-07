//! Direct routing for explicitly enrolled cgroups while a host VPN owns the default route.
//!
//! Packet classification covers existing sockets. Persistent slice anchors and a
//! readiness socket order enrollment before service execution. A separate DNS
//! listener keeps background queries independent of the desktop's resolver.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Component, Path};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// Fallible direct-network operations with underlying I/O diagnostics.
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
/// Reserved packet mark; never matches a whole UID or a desktop application slice.
pub const MARK: u32 = 0xca010000;
const TABLES: [u32; 2] = [51821, 51822];
const LOCAL_PRIORITY: u32 = 49;
const DIRECT_PRIORITY: u32 = 50;
const NFT_TABLE: &str = "toolbelt_direct_network";

/// Root-owned policy. Executables and destinations are explicit, never supplied by clients.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Policy {
    /// Readiness socket in a root-owned runtime directory.
    pub socket: String,
    /// dnsmasq's separately reloadable servers file.
    pub servers_file: String,
    /// Absolute iproute2 executable.
    pub ip: String,
    /// Absolute nft executable.
    pub nft: String,
    /// Absolute NetworkManager CLI executable.
    pub nmcli: String,
    /// Absolute systemctl executable.
    pub systemctl: String,
    /// Relative paths beneath the unified cgroup mount; children inherit enrollment.
    pub cgroup_paths: Vec<String>,
    /// Resolver cgroup whose upstream DNS must be routed directly without DNAT recursion.
    #[serde(default)]
    pub dns_cgroup: Option<String>,
    /// UIDs permitted to request readiness (this does not exempt their traffic).
    pub allowed_users: Vec<u32>,
    /// Additional direct interfaces, such as a fleet WireGuard interface.
    pub direct_interfaces: Vec<String>,
    /// Explicit public DNS servers, in addition to current uplink DNS.
    pub dns_servers: Vec<String>,
    /// Domain-specific direct DNS servers, independent of the desktop DNS configuration.
    pub dns_zones: BTreeMap<String, String>,
    /// Discover DNS from physical NetworkManager uplinks.
    pub network_manager_dns: bool,
}

fn invalid(message: impl Into<String>) -> Box<dyn std::error::Error + Send + Sync> {
    io::Error::new(io::ErrorKind::InvalidInput, message.into()).into()
}

fn token(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-/:@".contains(&b))
}

impl Policy {
    /// Validate paths and all tokens that will enter kernel command languages.
    pub fn validate(&self) -> Result<()> {
        for path in [
            &self.socket,
            &self.servers_file,
            &self.ip,
            &self.nft,
            &self.nmcli,
            &self.systemctl,
        ] {
            if !Path::new(path).is_absolute() || path.contains('\n') {
                return Err(invalid("direct-network policy requires absolute paths"));
            }
        }
        if self.cgroup_paths.is_empty() {
            return Err(invalid("direct-network requires explicit cgroups"));
        }
        for path in self.cgroup_paths.iter().chain(self.dns_cgroup.iter()) {
            if !token(path)
                || Path::new(path)
                    .components()
                    .any(|c| !matches!(c, Component::Normal(_)))
            {
                return Err(invalid("invalid relative cgroup path"));
            }
        }
        for device in &self.direct_interfaces {
            if !token(device) || device.contains('/') || device.len() > 15 {
                return Err(invalid("invalid direct interface"));
            }
        }
        for server in self.dns_servers.iter().chain(self.dns_zones.values()) {
            server
                .parse::<IpAddr>()
                .map_err(|_| invalid("DNS servers must be literal IP addresses"))?;
        }
        for zone in self.dns_zones.keys() {
            if zone.is_empty()
                || !zone
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".-".contains(&b))
            {
                return Err(invalid("invalid DNS zone"));
            }
        }
        Ok(())
    }
}

/// A safe, replayable main-table route on a direct interface.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Route {
    /// CIDR or `default`.
    pub destination: String,
    /// Interface, selected independently of Proton tunnel and dummy devices.
    pub device: String,
    /// Gateway, including scoped IPv6 link-local gateways via `device`.
    pub gateway: Option<String>,
    /// Preferred source address.
    pub source: Option<String>,
    /// Route metric.
    pub metric: u64,
    /// Connected routes are also kept reachable from desktop applications.
    pub local: bool,
    /// Gateways explicitly declared on-link by the producer.
    pub onlink: bool,
}

fn direct_devices(policy: &Policy, links: &Value) -> BTreeSet<String> {
    links
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|link| {
            let name = link["ifname"].as_str()?;
            let kind = link["linkinfo"]["info_kind"].as_str();
            let physical = link["link_type"] == "ether"
                && matches!(kind, None | Some("vlan" | "bridge" | "bond"));
            (token(name) && (physical || policy.direct_interfaces.iter().any(|d| d == name)))
                .then(|| name.to_owned())
        })
        .collect()
}

/// Select direct routes, rejecting VPN/dummy/container interfaces and malformed data.
pub fn select_routes(policy: &Policy, links: &Value, routes: &Value) -> Result<Vec<Route>> {
    let devices = direct_devices(policy, links);
    let mut selected = Vec::new();
    for route in routes
        .as_array()
        .ok_or_else(|| invalid("ip route did not return an array"))?
    {
        if route["type"].as_str().is_some_and(|kind| kind != "unicast") {
            continue;
        }
        let Some(device) = route["dev"].as_str().filter(|d| devices.contains(*d)) else {
            continue;
        };
        let destination = route["dst"].as_str().unwrap_or("default");
        let gateway = route["gateway"].as_str();
        let source = route["prefsrc"].as_str();
        if !token(destination)
            || gateway
                .into_iter()
                .chain(source)
                .any(|ip| ip.parse::<IpAddr>().is_err())
        {
            return Err(invalid("invalid address in direct route snapshot"));
        }
        selected.push(Route {
            destination: destination.to_owned(),
            device: device.to_owned(),
            gateway: gateway.map(str::to_owned),
            source: source.map(str::to_owned),
            metric: route["metric"].as_u64().unwrap_or(0),
            local: destination != "default" && gateway.is_none(),
            onlink: route["flags"]
                .as_array()
                .is_some_and(|flags| flags.iter().any(|f| f == "onlink")),
        });
    }
    // Populate connected routes before gateway routes. Sorting is stable for
    // equal preferences, retaining the producer's route ordering.
    selected.sort_by_key(|route| route.gateway.is_some());
    Ok(selected)
}

fn output(program: &str, args: &[String], input: Option<&str>) -> Result<String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if let Some(data) = input {
        child
            .stdin
            .take()
            .ok_or_else(|| invalid("missing command stdin"))?
            .write_all(data.as_bytes())?;
    }
    let result = child.wait_with_output()?;
    if !result.status.success() {
        return Err(invalid(format!(
            "{} {:?}: {}",
            program,
            args,
            String::from_utf8_lossy(&result.stderr).trim()
        )));
    }
    Ok(String::from_utf8(result.stdout)?)
}

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| (*v).to_owned()).collect()
}

fn json(program: &str, values: &[&str]) -> Result<Value> {
    Ok(serde_json::from_str(&output(
        program,
        &args(values),
        None,
    )?)?)
}

fn active_cgroups(policy: &Policy) -> BTreeMap<String, u64> {
    policy
        .cgroup_paths
        .iter()
        .chain(policy.dns_cgroup.iter())
        .filter_map(|path| {
            fs::metadata(Path::new("/sys/fs/cgroup").join(path))
                .ok()
                .filter(|metadata| metadata.is_dir())
                .map(|metadata| (path.clone(), metadata.ino()))
        })
        .collect()
}

fn nft_rules(
    policy: &Policy,
    paths: &[String],
    devices: &BTreeSet<String>,
    exists: bool,
) -> String {
    let mut text = String::new();
    if exists {
        text.push_str(&format!("delete table inet {NFT_TABLE}\n"));
    }
    text.push_str(&format!("table inet {NFT_TABLE} {{\n chain output {{\n type route hook output priority mangle; policy accept;\n"));
    // Passive TCP handshakes may be emitted from request sockets without the
    // listener's cgroup metadata. Restore the enrolled incoming flow's mark.
    text.push_str(&format!(
        "meta mark 0 ct mark {MARK} meta mark set {MARK}\n"
    ));
    for path in paths {
        let level = path.split('/').count();
        // Respect explicitly marked sockets used by other VPN/proxy engines.
        text.push_str(&format!(
            "meta mark 0 socket cgroupv2 level {level} \"{path}\" meta mark set {MARK}\n"
        ));
    }
    text.push_str(&format!("meta mark {MARK} ct mark set {MARK}\n }}\n chain incoming {{\n type filter hook prerouting priority mangle; policy accept;\n meta mark 0 ct mark {MARK} meta mark set {MARK}\n"));
    for path in paths {
        let level = path.split('/').count();
        text.push_str(&format!(
            "meta mark 0 socket cgroupv2 level {level} \"{path}\" counter meta mark set {MARK}\n"
        ));
    }
    text.push_str(&format!("meta mark {MARK} ct mark set {MARK}\n }}\n chain dns {{\n type nat hook output priority dstnat; policy accept;\n"));
    if let Some(path) = policy
        .dns_cgroup
        .as_ref()
        .filter(|path| paths.contains(path))
    {
        text.push_str(&format!(
            "socket cgroupv2 level {} \"{path}\" return\n",
            path.split('/').count()
        ));
    }
    text.push_str(&format!("meta mark {MARK} meta l4proto {{ tcp, udp }} th dport 53 dnat ip to 127.0.0.54:5354\n meta mark {MARK} meta l4proto {{ tcp, udp }} th dport 53 dnat ip6 to [::1]:5354\n }}\n chain source {{\n type nat hook postrouting priority srcnat; policy accept;\n"));
    for device in devices {
        // A new socket may initially choose the VPN/dummy address. Preserve
        // established conntrack mappings; normalize new direct flows only.
        text.push_str(&format!(
            "meta mark {MARK} oifname \"{device}\" masquerade\n"
        ));
    }
    text.push_str("}\n}\n");
    text
}

fn route_batch(routes: &[Route], table: u32) -> String {
    let mut text = format!("route flush table {table}\n");
    for route in routes {
        text.push_str(&format!(
            "route replace table {table} {}",
            route.destination
        ));
        if let Some(gateway) = &route.gateway {
            text.push_str(&format!(" via {gateway}"));
        }
        text.push_str(&format!(" dev {}", route.device));
        if let Some(source) = &route.source {
            text.push_str(&format!(" src {source}"));
        }
        text.push_str(&format!(" metric {}", route.metric));
        if route.onlink {
            text.push_str(" onlink");
        }
        text.push('\n');
    }
    // Never fall through to the VPN when the direct uplink has no route.
    text.push_str(&format!(
        "route replace unreachable default table {table} metric 4294967295\n"
    ));
    text
}

fn family_rules(policy: &Policy, family: &str) -> Result<Value> {
    json(&policy.ip, &[family, "-j", "rule", "show"])
}

fn own_rule(rule: &Value) -> bool {
    let table = rule_table(rule);
    TABLES.iter().any(|t| table == Some(u64::from(*t)))
        && (rule["priority"] == LOCAL_PRIORITY
            || (rule["priority"] == DIRECT_PRIORITY && rule["fwmark"] == format!("0x{MARK:x}")))
}

fn rule_table(rule: &Value) -> Option<u64> {
    rule["table"]
        .as_u64()
        .or_else(|| rule["table"].as_str()?.parse().ok())
}

fn delete_rule(policy: &Policy, family: &str, rule: &Value) -> Result<()> {
    let mut command = args(&[family, "rule", "del", "priority"]);
    command.push(rule["priority"].to_string());
    if rule["priority"] == DIRECT_PRIORITY {
        command.extend(args(&["fwmark", &MARK.to_string()]));
    } else {
        let destination = rule["dst"]
            .as_str()
            .ok_or_else(|| invalid("local direct rule lacks destination"))?;
        // iproute2's rule JSON separates the destination and its prefix length.
        // Passing just `dst` silently asks to delete a /32 or /128 instead.
        let destination = if let Some(prefix) = rule["dstlen"].as_u64() {
            format!("{destination}/{prefix}")
        } else {
            destination.to_owned()
        };
        command.extend(args(&["to", &destination]));
    }
    let table = rule_table(rule).ok_or_else(|| invalid("invalid direct route table"))?;
    command.extend(args(&["lookup", &table.to_string()]));
    output(&policy.ip, &command, None)?;
    Ok(())
}

fn check_reservations(policy: &Policy) -> Result<()> {
    for family in ["-4", "-6"] {
        for rule in family_rules(policy, family)?
            .as_array()
            .into_iter()
            .flatten()
        {
            let reserved_table =
                rule_table(rule).is_some_and(|table| TABLES.iter().any(|t| u64::from(*t) == table));
            if (rule["priority"] == LOCAL_PRIORITY
                || rule["priority"] == DIRECT_PRIORITY
                || reserved_table)
                && !own_rule(rule)
            {
                return Err(invalid(
                    "direct-network routing priorities 49/50 or tables 51821/51822 conflict with an existing owner",
                ));
            }
        }
    }
    Ok(())
}

fn switch_routes(policy: &Policy, family: &str, routes: &[Route]) -> Result<()> {
    let initial = family_rules(policy, family)?;
    let active = initial
        .as_array()
        .into_iter()
        .flatten()
        .find(|rule| rule["priority"] == DIRECT_PRIORITY && own_rule(rule))
        .and_then(rule_table);
    let table = if active == Some(u64::from(TABLES[0])) {
        TABLES[1]
    } else {
        TABLES[0]
    };
    // Recover an interrupted switch without ever flushing the table used by
    // the first live direct rule. This also handles a daemon restart safely.
    for rule in initial
        .as_array()
        .into_iter()
        .flatten()
        .filter(|rule| own_rule(rule) && rule_table(rule) == Some(u64::from(table)))
    {
        delete_rule(policy, family, rule)?;
    }
    output(
        &policy.ip,
        &args(&[family, "-batch", "-"]),
        Some(&route_batch(routes, table)),
    )?;
    let current = family_rules(policy, family)?;
    // New rules are installed while the previous complete table remains live.
    output(
        &policy.ip,
        &args(&[
            family,
            "rule",
            "add",
            "priority",
            &DIRECT_PRIORITY.to_string(),
            "fwmark",
            &MARK.to_string(),
            "lookup",
            &table.to_string(),
        ]),
        None,
    )?;
    let local: BTreeSet<_> = routes
        .iter()
        .filter(|r| r.local)
        .map(|r| r.destination.as_str())
        .collect();
    for destination in local {
        output(
            &policy.ip,
            &args(&[
                family,
                "rule",
                "add",
                "priority",
                &LOCAL_PRIORITY.to_string(),
                "to",
                destination,
                "lookup",
                &table.to_string(),
            ]),
            None,
        )?;
    }
    for rule in current
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| own_rule(r))
    {
        delete_rule(policy, family, rule)?;
    }
    Ok(())
}

fn dns_servers(policy: &Policy, devices: &BTreeSet<String>, links: &Value) -> Result<String> {
    let mut servers: BTreeSet<String> = policy.dns_servers.iter().cloned().collect();
    if policy.network_manager_dns {
        for device in devices {
            // Fleet VPN DNS is domain-specific below, never a public fallback.
            let virtual_kind = links
                .as_array()
                .into_iter()
                .flatten()
                .find(|link| link["ifname"] == *device)
                .and_then(|link| link["linkinfo"]["info_kind"].as_str());
            if virtual_kind == Some("wireguard") {
                continue;
            }
            let response = output(
                &policy.nmcli,
                &args(&[
                    "--wait",
                    "2",
                    "--escape",
                    "no",
                    "--get-values",
                    "IP4.DNS,IP6.DNS",
                    "device",
                    "show",
                    device,
                ]),
                None,
            )?;
            for line in response.lines() {
                if line.parse::<IpAddr>().is_ok() && !line.starts_with("127.") && line != "::1" {
                    servers.insert(line.to_owned());
                }
            }
        }
    }
    let mut text = servers
        .iter()
        .map(|s| format!("server={s}\n"))
        .collect::<String>();
    for (zone, server) in &policy.dns_zones {
        text.push_str(&format!("server=/{zone}/{server}\n"));
    }
    Ok(text)
}

#[derive(Default)]
struct State {
    snapshot: Option<String>,
    cgroups: BTreeMap<String, u64>,
    devices: BTreeSet<String>,
    nft_present: bool,
}

fn reconcile(policy: &Policy, state: &mut State) -> Result<()> {
    let links = json(&policy.ip, &["-j", "-d", "link", "show"])?;
    let devices = direct_devices(policy, &links);
    let v4 = select_routes(
        policy,
        &links,
        &json(&policy.ip, &["-4", "-j", "route", "show", "table", "main"])?,
    )?;
    let v6 = select_routes(
        policy,
        &links,
        &json(&policy.ip, &["-6", "-j", "route", "show", "table", "main"])?,
    )?;
    let snapshot = serde_json::to_string(&(&v4, &v6))?;
    check_reservations(policy)?;
    let rules_live = ["-4", "-6"]
        .into_iter()
        .map(|family| {
            family_rules(policy, family).map(|rules| {
                rules
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|r| r["priority"] == DIRECT_PRIORITY && own_rule(r))
            })
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .all(|live| live);
    if state.snapshot.as_ref() != Some(&snapshot) || !rules_live {
        switch_routes(policy, "-4", &v4)?;
        switch_routes(policy, "-6", &v6)?;
        state.snapshot = Some(snapshot);
    }
    let cgroups = active_cgroups(policy);
    let present = Command::new(&policy.nft)
        .args(["list", "table", "inet", NFT_TABLE])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?
        .success();
    if !state.nft_present || !present || cgroups != state.cgroups || devices != state.devices {
        let paths = cgroups.keys().cloned().collect::<Vec<_>>();
        output(
            &policy.nft,
            &args(&["-f", "-"]),
            Some(&nft_rules(policy, &paths, &devices, present)),
        )?;
        state.cgroups = cgroups;
        state.devices = devices.clone();
        state.nft_present = true;
    }
    let servers = dns_servers(policy, &devices, &links)?;
    if fs::read_to_string(&policy.servers_file).ok().as_deref() != Some(&servers) {
        let temp = format!("{}.new", policy.servers_file);
        fs::write(&temp, servers)?;
        fs::rename(temp, &policy.servers_file)?;
        // An absent DNS unit is normal during initial startup. No service
        // restart is needed when uplink DNS changes.
        let _ = Command::new(&policy.systemctl)
            .args([
                "kill",
                "--kill-whom=main",
                "--signal=HUP",
                "direct-network-dns.service",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    Ok(())
}

/// Run the root daemon. Kernel protection survives daemon failure or normal exit.
///
/// Only policy-selected cgroups are enrolled. Readiness clients cannot supply
/// routes, commands, interface names or new exemptions.
pub fn daemon(policy_path: &Path) -> Result<()> {
    if !rustix::process::geteuid().is_root() {
        return Err(invalid("direct-network daemon must run as root"));
    }
    let metadata = fs::metadata(policy_path)?;
    if metadata.uid() != 0 || metadata.mode() & 0o022 != 0 {
        return Err(invalid(
            "direct-network policy must be root-owned and not group/world writable",
        ));
    }
    let policy: Policy = serde_json::from_slice(&fs::read(policy_path)?)?;
    policy.validate()?;
    check_reservations(&policy)?;
    let listener = UnixListener::bind(&policy.socket)?;
    fs::set_permissions(&policy.socket, fs::Permissions::from_mode(0o666))?;
    listener.set_nonblocking(true)?;
    let mut state = State::default();
    let mut previous_error = String::new();
    loop {
        let result = reconcile(&policy, &mut state);
        if let Err(error) = &result {
            let message = error.to_string();
            if message != previous_error {
                eprintln!("direct-network not ready: {message}");
                previous_error = message;
            }
        } else {
            previous_error.clear();
        }
        for _ in 0..16 {
            let (mut stream, _) = match listener.accept() {
                Ok(connection) => connection,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(error.into()),
            };
            stream.set_write_timeout(Some(Duration::from_secs(1)))?;
            stream.set_read_timeout(Some(Duration::from_secs(1)))?;
            let credentials = rustix::net::sockopt::socket_peercred(&stream)?;
            let authorized = credentials.uid.is_root()
                || policy.allowed_users.contains(&credentials.uid.as_raw());
            let mut request = [0u8; 16];
            let count = stream.read(&mut request).unwrap_or(0);
            let routes_only = &request[..count] == b"routes\n" && credentials.uid.is_root();
            let supported = routes_only || &request[..count] == b"ready\n";
            // A slice may have appeared after the periodic snapshot. Enroll it
            // synchronously before acknowledging its anchor's startup barrier.
            let current = authorized
                && supported
                && reconcile(&policy, &mut state).is_ok()
                && (routes_only
                    || TcpStream::connect_timeout(
                        &SocketAddr::from(([127, 0, 0, 54], 5354)),
                        Duration::from_secs(1),
                    )
                    .is_ok());
            let message = if current { "ready\n" } else { "not-ready\n" };
            let _ = stream.write_all(message.as_bytes());
        }
        thread::sleep(Duration::from_secs(1));
    }
}

/// Wait a bounded time for the daemon's current route/DNS/cgroup readiness.
pub fn ready(socket: &Path) -> Result<()> {
    wait_ready(socket, false)
}

/// Root-only DNS bootstrap barrier, before the independent DNS listener starts.
pub fn routes_ready(socket: &Path) -> Result<()> {
    wait_ready(socket, true)
}

fn wait_ready(socket: &Path, routes_only: bool) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match request_ready(socket, routes_only, remaining.min(Duration::from_secs(2))) {
            Ok(()) => return Ok(()),
            Err(error) if Instant::now() >= deadline => return Err(error),
            Err(_) => thread::sleep(Duration::from_millis(100)),
        }
    }
}

fn request_ready(socket: &Path, routes_only: bool, timeout: Duration) -> Result<()> {
    let mut stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    stream.write_all(if routes_only { b"routes\n" } else { b"ready\n" })?;
    let mut response = String::new();
    stream.take(32).read_to_string(&mut response)?;
    if response == "ready\n" {
        Ok(())
    } else {
        Err(invalid("direct-network guard is not ready"))
    }
}
