//! Restart-safe coordination primitives. Consumers own policy, hosts and adapters.
//!
//! Existing JSON journals are retained verbatim, including pending request bodies.
//! A continuation acknowledgement establishes delivery, not completed execution.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub mod driver;
pub mod evidence;
pub mod snapshots;

/// An existing owner and its complete assignment set.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Packet {
    /// Stable packet identity.
    pub number: u32,
    /// Native owner session; never a replacement worker.
    #[serde(rename = "sessionID")]
    pub session_id: String,
    /// Declared execution host.
    #[serde(rename = "ownerHost")]
    pub owner_host: String,
    /// Preserved operator-visible title.
    pub title: String,
    /// Original assignments, supported linked repairs and linked releases.
    pub prs: Vec<String>,
    /// Linked repairs belonging to this same owner.
    #[serde(default, rename = "linkedRepairPRs")]
    pub linked_repairs: Vec<String>,
    /// Linked release work belonging to this same owner, distinct from repairs.
    #[serde(default, rename = "linkedReleasePRs")]
    pub linked_releases: Vec<String>,
    /// Shared producer owners.
    #[serde(default)]
    pub dependencies: Vec<u32>,
    /// Additional launch and ownership evidence retained during migration.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Existing campaign manifest with additive, ownership-preserving refreshes.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Manifest {
    /// Registered original owners.
    pub packets: Vec<Packet>,
    /// Total observed assignments.
    #[serde(rename = "assignedPRCount")]
    pub assignment_count: usize,
    /// Immutable original scope.
    #[serde(default, rename = "baselineAssignedPRCount")]
    pub baseline_count: Option<usize>,
    /// Explicit supported-repair count.
    #[serde(default, rename = "linkedRepairPRCount")]
    pub linked_count: Option<usize>,
    /// Explicit linked-release count, separate from baseline and repair scope.
    #[serde(default, rename = "linkedReleasePRCount")]
    pub release_count: Option<usize>,
    /// Historical manifest fields retained without reinterpretation.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl Manifest {
    /// Reject ambiguity, dropped assignments and owner replacement.
    pub fn validate(&self, previous: Option<&Self>) -> Result<(), String> {
        let mut numbers = BTreeSet::new();
        let mut sessions = BTreeSet::new();
        let mut urls = BTreeSet::new();
        let mut repairs = BTreeSet::new();
        let mut releases = BTreeSet::new();
        for packet in &self.packets {
            if packet.number == 0
                || !numbers.insert(packet.number)
                || !packet.session_id.starts_with("ses_")
                || !packet
                    .session_id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
                || !sessions.insert(&packet.session_id)
                || packet.title.trim().is_empty()
                || packet.owner_host.trim().is_empty()
                || packet.prs.is_empty()
            {
                return Err(format!("invalid or duplicate owner {}", packet.number));
            }
            for url in &packet.prs {
                if url.is_empty() || !urls.insert(url) {
                    return Err(format!("duplicate or empty assignment {url}"));
                }
            }
            for url in &packet.linked_repairs {
                if !packet.prs.contains(url) || !repairs.insert(url) {
                    return Err(format!("invalid linked repair {url}"));
                }
            }
            for url in &packet.linked_releases {
                if !packet.prs.contains(url) || repairs.contains(url) || !releases.insert(url) {
                    return Err(format!("invalid linked release {url}"));
                }
            }
        }
        if self.packets.is_empty()
            || urls.len() != self.assignment_count
            || self
                .linked_count
                .is_some_and(|count| count != repairs.len())
            || self
                .baseline_count
                .is_some_and(|count| count != urls.len() - repairs.len() - releases.len())
            || self
                .release_count
                .is_some_and(|count| count != releases.len())
        {
            return Err("assignment coverage does not match manifest counts".into());
        }
        for packet in &self.packets {
            if packet
                .dependencies
                .iter()
                .any(|id| *id == packet.number || !numbers.contains(id))
            {
                return Err(format!("invalid producer for owner {}", packet.number));
            }
        }
        if let Some(previous) = previous {
            previous.validate(None)?;
            if self.packets.len() != previous.packets.len()
                || self.baseline_count != previous.baseline_count
                || previous
                    .extra
                    .iter()
                    .any(|(key, value)| self.extra.get(key) != Some(value))
            {
                return Err("existing owner set or baseline scope changed".into());
            }
            for old in &previous.packets {
                let current = self
                    .packets
                    .iter()
                    .find(|packet| packet.number == old.number)
                    .ok_or_else(|| format!("owner {} was dropped", old.number))?;
                if current.session_id != old.session_id
                    || current.owner_host != old.owner_host
                    || current.title != old.title
                    || old
                        .extra
                        .iter()
                        .any(|(key, value)| current.extra.get(key) != Some(value))
                    || old
                        .dependencies
                        .iter()
                        .any(|id| !current.dependencies.contains(id))
                    || old.prs.iter().any(|url| !current.prs.contains(url))
                    || old.prs.iter().any(|url| {
                        old.linked_repairs.contains(url) != current.linked_repairs.contains(url)
                            || old.linked_releases.contains(url)
                                != current.linked_releases.contains(url)
                    })
                    || current.prs.iter().any(|url| {
                        !old.prs.contains(url)
                            && !current.linked_repairs.contains(url)
                            && !current.linked_releases.contains(url)
                    })
                {
                    return Err(format!(
                        "ownership changed or assignments dropped for {}",
                        old.number
                    ));
                }
            }
        }
        Ok(())
    }

    /// Count observed states without reclassifying immutable baseline work.
    pub fn coverage(&self, state: &Value) -> Result<Coverage, String> {
        self.validate(None)?;
        let mut coverage = Coverage::default();
        for packet in &self.packets {
            for url in &packet.prs {
                let status = state["prs"][url]["state"].as_str().unwrap_or("UNKNOWN");
                let (count, states) = if packet.linked_repairs.contains(url) {
                    (&mut coverage.repair_count, &mut coverage.repair_states)
                } else if packet.linked_releases.contains(url) {
                    (&mut coverage.release_count, &mut coverage.release_states)
                } else {
                    (&mut coverage.baseline_count, &mut coverage.baseline_states)
                };
                *count += 1;
                *states.entry(status.to_owned()).or_default() += 1;
            }
        }
        Ok(coverage)
    }
}

/// Manifest-bound coverage of original, linked-repair and linked-release work.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Coverage {
    /// Immutable baseline assignment count.
    #[serde(rename = "baselineAssignedPRCount")]
    pub baseline_count: usize,
    /// Supported-source repair assignment count.
    #[serde(rename = "linkedRepairPRCount")]
    pub repair_count: usize,
    /// Linked release assignment count.
    #[serde(rename = "linkedReleasePRCount")]
    pub release_count: usize,
    /// Baseline observations by native state; absent observations are UNKNOWN.
    #[serde(rename = "baselineStateCounts")]
    pub baseline_states: BTreeMap<String, usize>,
    /// Linked repair observations by native state.
    #[serde(rename = "linkedRepairStateCounts")]
    pub repair_states: BTreeMap<String, usize>,
    /// Linked release observations by native state.
    #[serde(rename = "linkedReleaseStateCounts")]
    pub release_states: BTreeMap<String, usize>,
}

/// Consumer-owned scheduler policy, independent of fleet names or model choices.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Policy {
    /// Current operator goal identity.
    pub goal_policy: String,
    /// Maximum concurrent existing owners per declared host.
    pub max_active_per_host: BTreeMap<String, usize>,
    /// Producer-first owners.
    #[serde(default)]
    pub producers: Vec<u32>,
    /// Stable ordering within a priority/fairness class.
    #[serde(default)]
    pub priority: Vec<u32>,
    /// First current-goal turns explicitly expedited by the consumer.
    #[serde(default)]
    pub expedited: Vec<u32>,
}

/// A native activity snapshot. Observation must include every configured host.
pub type Activity = BTreeMap<String, BTreeSet<u32>>;

/// Hash compatible with Python's sorted, ASCII JSON journals for ordinary data.
/// Imported request IDs and bodies must still be retained rather than rebuilt.
pub fn digest(value: &Value) -> String {
    fn string(value: &str) -> String {
        let mut ascii = String::new();
        for character in serde_json::to_string(value)
            .expect("string serialization")
            .chars()
        {
            if character <= '\x7e' {
                ascii.push(character);
            } else {
                let mut units = [0; 2];
                for unit in character.encode_utf16(&mut units) {
                    use std::fmt::Write as _;
                    let _ = write!(ascii, "\\u{unit:04x}");
                }
            }
        }
        ascii
    }
    fn canonical(value: &Value) -> String {
        match value {
            Value::Object(map) => format!(
                "{{{}}}",
                map.iter()
                    .collect::<BTreeMap<_, _>>()
                    .into_iter()
                    .map(|(key, value)| format!("{}:{}", string(key), canonical(value)))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            Value::Array(items) => format!(
                "[{}]",
                items.iter().map(canonical).collect::<Vec<_>>().join(",")
            ),
            Value::String(value) => string(value),
            Value::Number(number) if number.is_f64() => {
                let number = number.as_f64().expect("finite JSON number");
                let scientific = format!("{number:e}");
                let (mantissa, exponent) = scientific.split_once('e').expect("scientific number");
                let exponent: i32 = exponent.parse().expect("formatted exponent");
                if !(-4..16).contains(&exponent) {
                    format!("{mantissa}e{exponent:+03}")
                } else {
                    let text = number.to_string();
                    if text.contains('.') {
                        text
                    } else {
                        format!("{text}.0")
                    }
                }
            }
            _ => value.to_string(),
        }
    }
    format!("{:x}", Sha256::digest(canonical(value).as_bytes()))
}

/// Source/review/CI fingerprint, excluding poll timestamps and derived progress.
pub fn event_version(pr: &Value) -> String {
    let terminal = matches!(pr["state"].as_str(), Some("MERGED" | "CLOSED"));
    let ignored = [
        "capturedAt",
        "lastError",
        "lastErrorAt",
        "updatedAt",
        "deepAt",
        "classification",
        "progress",
    ];
    let terminal_ignored = [
        "base",
        "reportedBase",
        "checks",
        "mergeable",
        "mergeState",
        "reviewDecision",
    ];
    let entries = pr
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(key, _)| {
            !ignored.contains(&key.as_str())
                && !(terminal && terminal_ignored.contains(&key.as_str()))
        })
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    digest(&Value::Object(entries))
}

/// Full historical feedback identity used by terminal audits.
pub fn feedback_version(pr: &Value) -> String {
    let keys = [
        "url",
        "state",
        "head",
        "mergeCommit",
        "issueComments",
        "comments",
        "reviews",
        "threads",
        "inlineComments",
    ];
    digest(&Value::Object(
        keys.into_iter()
            .map(|key| (key.into(), pr[key].clone()))
            .collect(),
    ))
}

/// Whether an owner has an unadmitted event. Pending inputs require reconciliation.
pub fn needs_wake(packet: &Value) -> bool {
    packet["terminal"] != true
        && packet["pending"].is_null()
        && packet["version"] != packet["deliveredVersion"]
}

/// Classify observed progress without promoting a revision-stale worker claim.
pub fn progress(pr: &Value, reported: &Value) -> Value {
    let mut result = json!({"stage":"ready_for_work", "reason":"Reconcile findings and qualification", "head":pr["head"], "base":pr["base"]});
    let (stage, reason) = match pr["state"].as_str() {
        Some("MERGED") => (
            "merged",
            "Original merged; historical audit and any linked follow-up remain tracked",
        ),
        Some("CLOSED") => (
            "closed",
            "Verify the closure reason and historical finding disposition; keep the original closed",
        ),
        _ => {
            let stages = [
                "ready_for_work",
                "waiting_for_codex",
                "waiting_for_ci",
                "waiting_for_producer",
                "waiting_for_operator_or_maintainer",
                "ready_for_merge",
                "ready_for_close",
            ];
            if pr["head"].is_string()
                && pr["base"].is_string()
                && reported["head"] == pr["head"]
                && reported["base"] == pr["base"]
                && reported["stage"]
                    .as_str()
                    .is_some_and(|stage| stages.contains(&stage))
                && (!matches!(
                    reported["stage"].as_str(),
                    Some("ready_for_merge" | "ready_for_close")
                ) || present(&reported["evidence"]))
            {
                for key in ["stage", "reason", "evidence", "blockers"] {
                    if let Some(value) = reported.get(key) {
                        result[key] = value.clone();
                    }
                }
                result["source"] = json!("exact-revision worker checkpoint");
                return result;
            }
            if pr["state"] != "OPEN" {
                (
                    "waiting_for_operator_or_maintainer",
                    "Forge evidence unavailable; retry observation without declaring acceptance",
                )
            } else if pr["draft"] == true {
                (
                    "ready_for_work",
                    "Decide merge/repair/closure; qualify and mark ready when warranted, preserving any explicit maintainer hold",
                )
            } else if pr["unresolvedCodex"].as_u64().unwrap_or(0) > 0 {
                (
                    "ready_for_work",
                    "Inspect all current and historical Codex findings; resolution flags do not establish disposition",
                )
            } else {
                match pr["checks"]["state"]
                    .as_str()
                    .unwrap_or("")
                    .to_ascii_uppercase()
                    .as_str()
                {
                    "FAILURE" | "ERROR" => (
                        "ready_for_work",
                        "Investigate failing exact-revision qualification",
                    ),
                    "PENDING" | "EXPECTED" => (
                        "waiting_for_ci",
                        "Exact-revision CI remains pending; independent siblings can proceed",
                    ),
                    _ => (
                        "waiting_for_codex",
                        "Worker must reconcile current complete-candidate code review and all-history finding audit",
                    ),
                }
            }
        }
    };
    result["stage"] = json!(stage);
    result["reason"] = json!(reason);
    result
}

fn present(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::String(value) => !value.is_empty(),
        Value::Array(value) => !value.is_empty(),
        Value::Object(value) => !value.is_empty(),
        Value::Number(value) => value.as_f64() != Some(0.0),
    }
}

fn journal_packet(state: &mut Value, number: u32) -> Result<&mut Value, String> {
    if !state.is_object() || !state["packets"].is_object() || !state["prs"].is_object() {
        return Err("campaign journal must contain packets and prs objects".into());
    }
    let entry = state["packets"]
        .as_object_mut()
        .expect("validated object")
        .entry(number.to_string())
        .or_insert_with(|| json!({}));
    if !entry.is_object() {
        return Err(format!("invalid journal packet {number}"));
    }
    for key in [
        "auditedFeedback",
        "auditedCheckpoints",
        "auditedDependencies",
    ] {
        if !entry[key].is_null() && !entry[key].is_object() {
            return Err(format!("invalid {key} journal field for {number}"));
        }
    }
    if !entry["pending"].is_null()
        && (!entry["pending"].is_object()
            || !entry["pending"]["body"].is_object()
            || !entry["pending"]["body"]["id"].as_str().is_some_and(|id| {
                !id.is_empty()
                    && id
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
            }))
    {
        return Err(format!("invalid pending request for {number}"));
    }
    Ok(entry)
}

fn validate_report_urls(report: &Value, number: u32) -> Result<(), String> {
    let mut urls = BTreeSet::new();
    for key in ["prs", "linkedReleasePRs"] {
        if !report[key].is_null() && !report[key].is_array() {
            return Err(format!("invalid worker report PR collection for {number}"));
        }
        for item in report[key].as_array().into_iter().flatten() {
            let url = item["url"]
                .as_str()
                .filter(|url| !url.is_empty())
                .ok_or_else(|| format!("missing worker report URL for {number}"))?;
            if !urls.insert(url) {
                return Err(format!("duplicate worker report URL for {number}: {url}"));
            }
        }
    }
    Ok(())
}

/// Refresh observations and audits while retaining every historical journal field.
/// Reports and dependency digests are supplied by the consumer's source-bound I/O.
pub fn refresh(
    manifest: &Manifest,
    state: &mut Value,
    policy: &Policy,
    reports: &BTreeMap<u32, Value>,
    audits: &BTreeMap<u32, Value>,
    dependencies: &BTreeMap<u32, String>,
) -> Result<(), String> {
    let mut candidate = state.clone();
    refresh_in_place(
        manifest,
        &mut candidate,
        policy,
        reports,
        audits,
        dependencies,
    )?;
    *state = candidate;
    Ok(())
}

fn refresh_in_place(
    manifest: &Manifest,
    state: &mut Value,
    policy: &Policy,
    reports: &BTreeMap<u32, Value>,
    audits: &BTreeMap<u32, Value>,
    dependencies: &BTreeMap<u32, String>,
) -> Result<(), String> {
    let retained = state
        .get("retainedManifest")
        .map(|value| {
            serde_json::from_value::<Manifest>(value.clone())
                .map_err(|error| format!("invalid retained manifest: {error}"))
        })
        .transpose()?;
    manifest.validate(retained.as_ref())?;
    // Imported journals must not lose their already-observed ownership scope.
    if !state["prs"].is_object()
        || !state["packets"].is_object()
        || state["prs"]
            .as_object()
            .into_iter()
            .flatten()
            .any(|(url, _)| {
                !manifest
                    .packets
                    .iter()
                    .any(|packet| packet.prs.contains(url))
            })
        || state["packets"]
            .as_object()
            .into_iter()
            .flatten()
            .any(|(number, _)| {
                !manifest
                    .packets
                    .iter()
                    .any(|packet| packet.number.to_string() == *number)
            })
    {
        return Err("manifest drops retained journal scope".into());
    }
    for packet in &manifest.packets {
        journal_packet(state, packet.number)?;
        for url in &packet.prs {
            let pr = state["prs"]
                .as_object_mut()
                .expect("validated object")
                .entry(url)
                .or_insert_with(|| json!({"url":url}));
            if !pr.is_object() {
                return Err(format!("invalid PR journal record {url}"));
            }
            if pr.get("url").is_none() {
                pr["url"] = json!(url);
            }
            if pr["url"].as_str() != Some(url) {
                return Err(format!("PR journal identity differs from assignment {url}"));
            }
        }
        let prs: Vec<_> = packet
            .prs
            .iter()
            .map(|url| state["prs"][url].clone())
            .collect();
        let ps = journal_packet(state, packet.number)?;
        ps["openPRs"] = json!(prs.iter().filter(|pr| pr["state"] == "OPEN").count());
        let deps: BTreeMap<_, _> = packet
            .dependencies
            .iter()
            .filter_map(|id| dependencies.get(id).map(|hash| (id.to_string(), hash)))
            .collect();
        let dependency_version = digest(&json!({"producers":packet.dependencies,"evidence":deps}));
        let mut inputs = json!({"prs":prs.iter().map(event_version).collect::<Vec<_>>(), "dependencies":deps, "goalPolicy":policy.goal_policy});
        // A declared producer is useful new input even before its first handoff.
        // Preserve historical fingerprints for packets without dependencies.
        if !packet.dependencies.is_empty() {
            inputs["producers"] = json!(packet.dependencies);
        }
        let observation = json!(digest(&inputs));
        // Bind an imported delivery only to its retained, unchanged observation.
        // A journal without that evidence requires a fresh delivered turn.
        if ps["deliveredObservationVersion"].is_null()
            && ps["deliveredVersion"].is_string()
            && ps["deliveredVersion"] == ps["version"]
            && ps["observationVersion"].is_string()
        {
            ps["deliveredObservationVersion"] = ps["observationVersion"].clone();
        }
        // Migrate only progress already included in the exact delivered event,
        // before adopting a new ready-for-work report in this refresh.
        if ps["deliveredProgressVersion"].is_null()
            && ps["deliveredVersion"].is_string()
            && ps["deliveredVersion"] == ps["version"]
            && present(&ps["progressVersion"])
        {
            ps["deliveredProgressVersion"] = ps["progressVersion"].clone();
        }
        // Keep the waiting obligation when a follow-up makes the old report
        // stale. Only a report bound to the current delivery can release it.
        if ps["workerStatus"]["status"] == "waiting" {
            ps["waitingReportPending"] = json!(true);
        }
        if let Some(report) = reports.get(&packet.number) {
            if report["packet"].as_u64() != Some(u64::from(packet.number))
                || report["schemaVersion"] != 1
            {
                return Err(format!(
                    "worker status identity mismatch for {}",
                    packet.number
                ));
            }
            validate_report_urls(report, packet.number)?;
            ps["workerStatus"] = report.clone();
            if report["eventVersion"].is_string()
                && report["eventVersion"] == ps["deliveredVersion"]
                && ps["deliveredObservationVersion"] == observation
            {
                if matches!(
                    report["status"].as_str(),
                    Some("waiting" | "ready_for_work" | "merged" | "closed" | "complete")
                ) {
                    ps["waitingReportPending"] = json!(report["status"] == "waiting");
                }
                if report["status"] == "ready_for_work" {
                    ps["progressVersion"] = json!(digest(report));
                }
                for pr in &prs {
                    let item = report["prs"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .chain(report["linkedReleasePRs"].as_array().into_iter().flatten())
                        .find(|item| item["url"] == pr["url"]);
                    if let Some(item) = item {
                        let checkpoint = digest(&json!([report["eventVersion"], item]));
                        let terminal_claim = pr["state"] == "MERGED"
                            || (pr["state"] == "CLOSED" && present(&item["disposition"]));
                        let url = pr["url"].as_str().ok_or("missing journal URL")?;
                        if terminal_claim
                            && pr["head"].is_string()
                            && item["head"] == pr["head"]
                            && item["auditComplete"] == true
                            && present(&item["evidence"])
                            && ps["auditedCheckpoints"][url] != checkpoint
                        {
                            ps["auditedFeedback"][url] = json!(feedback_version(pr));
                            ps["auditedCheckpoints"][url] = json!(checkpoint);
                            if !ps["auditedDependencies"].is_object() {
                                ps["auditedDependencies"] = json!({});
                            }
                            ps["auditedDependencies"][url] = json!(dependency_version);
                        }
                    }
                }
            }
        }
        ps["observationVersion"] = observation;
        ps["versionInputs"] = inputs.clone();
        let mut version_inputs = inputs;
        version_inputs["progressVersion"] = ps["progressVersion"].clone();
        for key in ["recoveryVersion", "recheckVersion"] {
            if present(&ps[key]) {
                version_inputs[key] = ps[key].clone();
            }
        }
        ps["version"] = json!(digest(&version_inputs));
        if let Some(audit) = audits.get(&packet.number) {
            if audit["packet"].as_u64() != Some(u64::from(packet.number)) {
                return Err("historical audit identity mismatch".into());
            }
            for item in audit["prs"].as_array().into_iter().flatten() {
                if present(&item["evidence"])
                    && present(&item["feedbackVersion"])
                    && (packet.dependencies.is_empty()
                        || item["dependencyVersion"].as_str() == Some(&dependency_version))
                {
                    let url = item["url"].as_str().ok_or("audit is missing URL")?;
                    if prs.iter().any(|pr| {
                        pr["url"] == url
                            && item["feedbackVersion"].as_str()
                                == Some(feedback_version(pr).as_str())
                    }) {
                        ps["auditedFeedback"][url] = item["feedbackVersion"].clone();
                        if !ps["auditedDependencies"].is_object() {
                            ps["auditedDependencies"] = json!({});
                        }
                        ps["auditedDependencies"][url] = json!(dependency_version);
                    }
                }
            }
        }
        let backlog = prs
            .iter()
            .filter(|pr| {
                matches!(pr["state"].as_str(), Some("MERGED" | "CLOSED"))
                    && (ps["auditedFeedback"][pr["url"].as_str().unwrap_or("")].as_str()
                        != Some(feedback_version(pr).as_str())
                        || (!packet.dependencies.is_empty()
                            && ps["auditedDependencies"][pr["url"].as_str().unwrap_or("")]
                                .as_str()
                                != Some(&dependency_version)))
            })
            .count();
        ps["terminal"] = json!(
            backlog == 0
                && ps["waitingReportPending"] != true
                && ps["workerStatus"]["status"] != "waiting"
                && !undelivered_progress(ps)
                && prs
                    .iter()
                    .all(|pr| matches!(pr["state"].as_str(), Some("MERGED" | "CLOSED")))
        );
        ps["auditBacklog"] = json!(backlog);
        let current_report = ps["workerStatus"]["eventVersion"].is_string()
            && ps["workerStatus"]["eventVersion"] == ps["deliveredVersion"]
            && ps["deliveredObservationVersion"] == ps["observationVersion"];
        let reported: Vec<_> = if current_report {
            validate_report_urls(&ps["workerStatus"], packet.number)?;
            ["prs", "linkedReleasePRs"]
                .into_iter()
                .flat_map(|key| {
                    ps["workerStatus"][key]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .cloned()
                })
                .collect()
        } else {
            Vec::new()
        };
        let feedback = ps["auditedFeedback"].clone();
        let audited_dependencies = ps["auditedDependencies"].clone();
        for url in &packet.prs {
            let pr = &mut state["prs"][url];
            let item = reported
                .iter()
                .find(|item| item["url"] == *url)
                .unwrap_or(&Value::Null);
            pr["progress"] = progress(pr, item);
            pr["classification"] = pr["progress"]["stage"].clone();
            pr["progress"]["auditComplete"] = json!(
                feedback[url].as_str() == Some(feedback_version(pr).as_str())
                    && (packet.dependencies.is_empty()
                        || audited_dependencies[url].as_str() == Some(&dependency_version))
            );
        }
    }
    state["retainedManifest"] =
        serde_json::to_value(manifest).map_err(|error| error.to_string())?;
    state["coverage"] =
        serde_json::to_value(manifest.coverage(state)?).map_err(|error| error.to_string())?;
    Ok(())
}

fn timestamp(value: &Value) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(value.as_str()?)
        .ok()
        .map(|time| time.timestamp())
}

fn undelivered_progress(ps: &Value) -> bool {
    present(&ps["progressVersion"]) && ps["progressVersion"] != ps["deliveredProgressVersion"]
}

/// One bounded follow-up for each unchanged waiting checkpoint, with an injected clock.
pub fn schedule_recheck(ps: &mut Value, policy: &Policy, now: i64) -> bool {
    if ps["terminal"] == true || present(&ps["pending"]) || needs_wake(ps) {
        return false;
    }
    let report = &ps["workerStatus"];
    if report["status"] != "waiting" || report["eventVersion"] != ps["deliveredVersion"] {
        return false;
    }
    let checkpoint = digest(report);
    let Some(entered) = timestamp(&report["updatedAt"])
        .or_else(|| timestamp(&report["at"]))
        .or_else(|| timestamp(&ps["deliveredPreparedAt"]))
    else {
        return false;
    };
    let delay = report["retryAfterSeconds"]
        .as_i64()
        .unwrap_or(900)
        .clamp(600, 1800);
    if ps["recheckedReport"] == checkpoint || now < entered.saturating_add(delay) {
        return false;
    }
    ps["recheckedReport"] = json!(checkpoint);
    ps["recheckVersion"] = json!(digest(&json!([
        policy.goal_policy,
        ps["observationVersion"],
        checkpoint,
        "terminal-disposition-followup"
    ])));
    true
}

/// Recover an idle turn missing a valid checkpoint at most three times per source.
pub fn record_idle_recovery(ps: &mut Value, idle: &Value, now: i64) -> bool {
    if ps["terminal"] == true || present(&ps["pending"]) || ps["version"] != ps["deliveredVersion"]
    {
        return false;
    }
    let (Some(started), Some(id), Some(idle_ms)) = (
        ps.get("deliveredSubmissionAttemptAt")
            .filter(|value| !value.is_null())
            .unwrap_or(&ps["deliveredAt"])
            .as_str()
            .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok()),
        idle["id"].as_str(),
        idle["time"]["created"].as_i64(),
    ) else {
        return false;
    };
    let Some(idle_at) = chrono::DateTime::from_timestamp_millis(idle_ms) else {
        return false;
    };
    if idle_at < started || now.saturating_mul(1000).saturating_sub(idle_ms) < 300_000 {
        return false;
    }
    let report = &ps["workerStatus"];
    let valid = matches!(
        report["status"].as_str(),
        Some("waiting" | "ready_for_work" | "merged" | "closed" | "complete")
    ) && !(matches!(
        report["status"].as_str(),
        Some("merged" | "closed" | "complete")
    ) && ps["auditBacklog"].as_u64().unwrap_or(0) > 0);
    if report["eventVersion"] == ps["deliveredVersion"] && valid {
        return false;
    }
    let source = ps["observationVersion"].clone();
    if !ps["idleRecovery"].is_object() || ps["idleRecovery"]["source"] != source {
        ps["idleRecovery"] = json!({"source":source,"attempts":0,"idleIDs":[]});
    }
    if ps["idleRecovery"]["idleIDs"]
        .as_array()
        .is_some_and(|ids| ids.iter().any(|value| value == id))
    {
        return false;
    }
    let attempts = ps["idleRecovery"]["attempts"].as_u64().unwrap_or(0);
    if attempts >= 3 {
        ps["continuationBlocker"] = json!({"resource":"worker checkpoint/admission", "owner":"existing owner session", "releaseCondition":"new source/review/CI/dependency evidence or a usable worker checkpoint", "reason":"Three bounded continuations ended without a checkpoint; retained unfinished"});
        return false;
    }
    let Some(ids) = ps["idleRecovery"]["idleIDs"].as_array_mut() else {
        return false;
    };
    ids.push(json!(id));
    ps["idleRecovery"]["attempts"] = json!(attempts + 1);
    ps["recoveryVersion"] = json!(digest(&json!([source, id, "missing-idle-checkpoint"])));
    ps["lastIdleOutcome"] = idle["outcome"].clone();
    true
}

/// Select existing owners with bounded capacity and oldest-admission fairness.
pub fn select<'a>(
    manifest: &'a Manifest,
    states: &Value,
    active: &Activity,
    policy: &Policy,
) -> Result<Vec<&'a Packet>, String> {
    manifest.validate(None)?;
    if policy.goal_policy.trim().is_empty()
        || policy.max_active_per_host.is_empty()
        || policy
            .max_active_per_host
            .values()
            .any(|limit| *limit == 0 || *limit > 64)
        || active.keys().ne(policy.max_active_per_host.keys())
        || manifest
            .packets
            .iter()
            .any(|packet| !active.contains_key(&packet.owner_host))
    {
        return Err("invalid or incomplete host capacity/activity observation".into());
    }
    for (host, owners) in active {
        if owners.iter().any(|id| {
            !manifest
                .packets
                .iter()
                .any(|packet| packet.number == *id && &packet.owner_host == host)
        }) {
            return Err(format!("unregistered or wrong-host execution on {host}"));
        }
    }
    let mut ordered: Vec<_> = manifest.packets.iter().collect();
    ordered.sort_by_key(|packet| {
        let state = &states[packet.number.to_string()];
        let first_goal = state["goalAckVersion"].as_str() != Some(&policy.goal_policy);
        let expedited = policy
            .expedited
            .iter()
            .position(|id| *id == packet.number)
            .filter(|_| first_goal);
        let class = if policy.producers.contains(&packet.number) {
            0
        } else if expedited.is_some() {
            1
        } else if state["deliveredVersion"].is_null() {
            2
        } else if first_goal {
            3
        } else {
            4
        };
        let oldest = if class == 4 {
            state["deliveredPreparedAt"]
                .as_str()
                .or_else(|| state["deliveredAt"].as_str())
                .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                .map(|time| time.with_timezone(&chrono::Utc))
        } else {
            None
        };
        (
            class,
            expedited.unwrap_or(0),
            oldest,
            policy
                .priority
                .iter()
                .position(|id| *id == packet.number)
                .unwrap_or(usize::MAX),
            packet.number,
        )
    });
    let mut counts: BTreeMap<_, _> = active
        .iter()
        .map(|(host, owners)| (host.clone(), owners.len()))
        .collect();
    let mut selected = Vec::new();
    for packet in ordered {
        let host = &packet.owner_host;
        if !active[host].contains(&packet.number)
            && counts[host] < policy.max_active_per_host[host]
            && needs_wake(&states[packet.number.to_string()])
        {
            selected.push(packet);
            *counts.get_mut(host).expect("validated host") += 1;
        }
    }
    Ok(selected)
}

/// Preserve pending bodies on restart; prepare fresh inputs only once.
pub fn prepare(
    state: &mut Value,
    packet: &Packet,
    policy: &Policy,
    text: &str,
    at: &str,
) -> Result<Value, String> {
    let ps = journal_packet(state, packet.number)?;
    if !ps["pending"].is_null() {
        return Ok(ps["pending"]["body"].clone());
    }
    if !needs_wake(ps) || text.trim().is_empty() {
        return Err("owner has no unadmitted event or continuation text is empty".into());
    }
    let version = ps["version"].clone();
    let generation = match ps.get("admissionGeneration") {
        None | Some(Value::Null) => 0,
        Some(value) => value.as_u64().ok_or("invalid admission generation")?,
    }
    .checked_add(1)
    .ok_or("admission generation exhausted")?;
    let hash = digest(&json!([
        packet.session_id,
        version,
        policy.goal_policy,
        generation
    ]));
    let body = json!({"id":format!("msg_{}", &hash[..32]), "text":text, "delivery":"steer", "resume":true,
        "metadata":{"coordination":"native-campaign-v1", "packet":packet.number, "eventVersion":version}});
    ps["admissionGeneration"] = json!(generation);
    ps["pending"] = json!({"version":version, "observationVersion":ps["observationVersion"], "progressVersion":ps["progressVersion"], "goalPolicy":policy.goal_policy,"body":body,"preparedAt":at});
    Ok(body)
}

/// Adopt a durable inbox/message receipt without interpreting it as execution success.
pub fn acknowledge(state: &mut Value, number: u32, id: &str, at: &str) -> Result<(), String> {
    let ps = journal_packet(state, number)?;
    let pending = ps["pending"].clone();
    if pending["body"]["id"].as_str() != Some(id) {
        return Err("acknowledgement does not match the exact pending input".into());
    }
    ps["deliveredVersion"] = pending["version"].clone();
    ps["deliveredObservationVersion"] = pending["observationVersion"].clone();
    ps["deliveredAt"] = json!(at);
    ps["deliveredPreparedAt"] = pending["preparedAt"].clone();
    ps["deliveredSubmissionAttemptAt"] = pending["submissionAttemptAt"].clone();
    ps["deliveredProgressVersion"] = pending["progressVersion"].clone();
    ps["goalAckVersion"] = pending["goalPolicy"].clone();
    ps["pending"] = Value::Null;
    Ok(())
}

/// Verify audited terminal coverage, complete observation and idle execution.
pub fn finished(
    manifest: &Manifest,
    state: &Value,
    active: &Activity,
    observation_ok: bool,
) -> bool {
    observation_ok
        && manifest.validate(None).is_ok()
        && manifest
            .packets
            .iter()
            .all(|packet| active.contains_key(&packet.owner_host))
        && !active.is_empty()
        && active.values().all(BTreeSet::is_empty)
        && manifest.packets.iter().all(|packet| {
            let ps = &state["packets"][packet.number.to_string()];
            ps["terminal"] == true
                && ps["waitingReportPending"] != true
                && ps["workerStatus"]["status"] != "waiting"
                && !undelivered_progress(ps)
                && ps["pending"].is_null()
                && ps["pendingCompaction"].is_null()
                && packet.prs.iter().all(|url| {
                    let pr = &state["prs"][url];
                    matches!(pr["state"].as_str(), Some("MERGED" | "CLOSED"))
                        && pr["lastError"].is_null()
                        && ps["auditedFeedback"][url].as_str()
                            == Some(feedback_version(pr).as_str())
                })
        })
}

/// Persistent campaign lock shared with legacy coordinators; never unlink its anchor.
pub struct Lease(File);

impl Lease {
    /// Acquire the existing campaign writer lock without waiting or replacing it.
    pub fn acquire(path: &Path) -> io::Result<Self> {
        if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "campaign lock is a symlink",
            ));
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
            .mode(0o600)
            .open(path)?;
        if !file.metadata()?.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "campaign lock is not a regular file",
            ));
        }
        file.try_lock_exclusive()?;
        Ok(Self(file))
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}

/// Atomic, fsynced, mode-0600 JSON publication without clobbering a symlink.
pub fn atomic_json(path: &Path, value: &Value) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    if fs::symlink_metadata(path).is_ok_and(|metadata| !metadata.is_file()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "state destination is not a regular file",
        ));
    }
    let mut file = tempfile::Builder::new()
        .prefix(".native-state-")
        .tempfile_in(parent)?;
    (|| {
        serde_json::to_writer_pretty(&mut file, value)?;
        file.write_all(b"\n")?;
        file.as_file().sync_all()?;
        file.as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))?;
        file.persist(path).map_err(|error| error.error)?;
        File::open(parent)?.sync_all()
    })()
}
