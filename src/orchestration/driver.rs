//! Bounded native owner observation and durable prompt admission.
use super::{
    Activity, Manifest, Packet, Policy, acknowledge, digest, prepare, record_idle_recovery,
    schedule_recheck, select,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

/// A decoded OpenCode response. A 404 remains distinct from transport failure.
pub struct Reply {
    /// Native HTTP status.
    pub status: u16,
    /// The response's `data` value, not the envelope.
    pub data: Value,
}

impl Reply {
    fn checked(self) -> io::Result<Value> {
        if !(200..300).contains(&self.status) {
            return Err(invalid(format!("OpenCode HTTP {}", self.status)));
        }
        Ok(self.data)
    }
}

/// Retained session selectors, captured before cutover rather than changed at admission.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    /// Exact provider, model and variant.
    pub model: Value,
    /// Existing permission-bearing agent.
    pub agent: String,
    /// Exact session permission overrides. Agent policy is verified by the consumer adapter.
    pub permissions: Value,
}

/// One expectation per existing owner.
pub type Expectations = BTreeMap<u32, Identity>;

/// Consumer-owned native transport and private state publication.
pub trait Adapter {
    /// Call the declared host's existing service without moving credentials.
    fn api(
        &mut self,
        host: &str,
        method: &str,
        path: &str,
        body: Option<&Value>,
    ) -> io::Result<Reply>;
    /// Fsync the complete journal under the shared writer lease.
    fn persist(&mut self, state: &Value) -> io::Result<()>;
    /// Publish/verify the owner snapshot before returning consumer-owned steering text.
    fn continuation(&mut self, packet: &Packet, state: &Value) -> io::Result<String>;
}

/// Full owner observation, including queued inputs and context pressure.
pub struct Observation {
    /// Actual foreground execution, used for completion.
    pub active: Activity,
    /// Foreground or queued owners, used for capacity accounting.
    pub occupied: Activity,
    /// Exact owner inbox contents.
    pub inboxes: BTreeMap<u32, Vec<Value>>,
    /// Recent messages used for recovery and context accounting.
    pub messages: BTreeMap<u32, Vec<Value>>,
    /// Last observed input/cache tokens per owner.
    pub context_tokens: BTreeMap<u32, u64>,
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
fn request(adapter: &mut impl Adapter, packet: &Packet, suffix: &str) -> io::Result<Value> {
    adapter
        .api(
            &packet.owner_host,
            "GET",
            &format!("/api/session/{}{suffix}", packet.session_id),
            None,
        )?
        .checked()
}

/// Validate every host and owner before any new admission. Mirrors cannot execute.
pub fn observe(
    manifest: &Manifest,
    policy: &Policy,
    expected: &Expectations,
    adapter: &mut impl Adapter,
) -> io::Result<Observation> {
    manifest.validate(None).map_err(invalid)?;
    if expected.len() != manifest.packets.len()
        || manifest
            .packets
            .iter()
            .any(|p| !expected.contains_key(&p.number))
    {
        return Err(invalid(
            "complete retained session expectations are required",
        ));
    }
    let mut raw = BTreeMap::new();
    for host in policy.max_active_per_host.keys() {
        let data = adapter
            .api(host, "GET", "/api/session/active", None)?
            .checked()?;
        if !data.is_object() {
            return Err(invalid(format!(
                "incomplete activity observation on {host}"
            )));
        }
        raw.insert(host.clone(), data);
    }
    let mut active: Activity = raw
        .keys()
        .map(|host| (host.clone(), BTreeSet::new()))
        .collect();
    let mut inboxes = BTreeMap::new();
    let mut messages = BTreeMap::new();
    let mut context_tokens = BTreeMap::new();
    for packet in &manifest.packets {
        for (host, data) in &raw {
            if data.get(&packet.session_id).is_some() {
                if *host != packet.owner_host {
                    return Err(invalid(format!(
                        "owner {} executing on mirror {host}; owner is {}",
                        packet.number, packet.owner_host
                    )));
                }
                active
                    .get_mut(host)
                    .expect("observed host")
                    .insert(packet.number);
            }
        }
        if !raw.contains_key(&packet.owner_host) {
            return Err(invalid("missing owner host observation"));
        }
        for host in raw.keys().filter(|host| **host != packet.owner_host) {
            let reply = adapter.api(
                host,
                "GET",
                &format!("/api/session/{}/inbox", packet.session_id),
                None,
            )?;
            if reply.status != 404 {
                let data = reply.checked()?;
                let inbox = data
                    .as_array()
                    .ok_or_else(|| invalid("incomplete mirror inbox observation"))?;
                if !inbox.is_empty() {
                    return Err(invalid(format!(
                        "owner {} has pending input on mirror {host}; reconcile the exact input before admission",
                        packet.number
                    )));
                }
            }
        }
        let info = request(adapter, packet, "")?;
        let identity = &expected[&packet.number];
        if info["id"] != packet.session_id
            || info["title"] != packet.title
            || info["model"] != identity.model
            || info["agent"].as_str() != Some(&identity.agent)
            || info["permissions"] != identity.permissions
        {
            return Err(invalid(format!(
                "owner {} identity/model/permission selectors changed",
                packet.number
            )));
        }
        let inbox = request(adapter, packet, "/inbox")?
            .as_array()
            .cloned()
            .ok_or_else(|| invalid("incomplete inbox observation"))?;
        let recent = request(adapter, packet, "/message?limit=12&order=desc")?
            .as_array()
            .cloned()
            .ok_or_else(|| invalid("incomplete context observation"))?;
        let physical = recent.iter().find(|m| {
            m["type"] == "assistant"
                && (m["tokens"]["input"].as_u64().unwrap_or(0) > 0
                    || m["tokens"]["cache"]["read"].as_u64().unwrap_or(0) > 0)
        });
        let compacted = physical.is_some_and(|m| {
            recent.iter().any(|c| {
                c["type"] == "compaction"
                    && c["status"] == "completed"
                    && c["time"]["created"].as_u64().unwrap_or(0)
                        > m["time"]["created"].as_u64().unwrap_or(u64::MAX)
            })
        });
        let tokens = physical
            .filter(|_| !compacted)
            .map(|m| {
                [
                    m["tokens"]["input"].as_u64().unwrap_or(0),
                    m["tokens"]["cache"]["read"].as_u64().unwrap_or(0),
                    m["tokens"]["cache"]["write"].as_u64().unwrap_or(0),
                ]
                .into_iter()
                .fold(0u64, u64::saturating_add)
            })
            .unwrap_or(0);
        if tokens >= 300_000 {
            return Err(invalid(format!(
                "owner {} context ceiling reached: {tokens}",
                packet.number
            )));
        }
        inboxes.insert(packet.number, inbox);
        messages.insert(packet.number, recent);
        context_tokens.insert(packet.number, tokens);
    }
    let mut occupied = active.clone();
    for packet in &manifest.packets {
        if !inboxes[&packet.number].is_empty() {
            occupied
                .get_mut(&packet.owner_host)
                .expect("validated host")
                .insert(packet.number);
        }
    }
    // Reuse scheduler validation even when no work is currently ready.
    select(manifest, &json!({}), &occupied, policy).map_err(invalid)?;
    Ok(Observation {
        active,
        occupied,
        inboxes,
        messages,
        context_tokens,
    })
}

fn verify_input(item: &Value, body: &Value, packet: &Packet) -> io::Result<()> {
    if item["id"] != body["id"]
        || (!item["sessionID"].is_null() && item["sessionID"] != packet.session_id)
        || item["type"] != "user"
    {
        return Err(invalid("pending input receipt identity/type differs"));
    }
    let payload = item.get("payload").unwrap_or(item);
    if payload["text"] != body["text"] || payload["metadata"] != body["metadata"] {
        return Err(invalid(
            "pending input receipt body differs; retain both for inspection",
        ));
    }
    for key in ["files", "agents", "skills"] {
        if !body[key].is_null() && payload[key] != body[key] {
            return Err(invalid("pending input attachment receipt differs"));
        }
    }
    Ok(())
}

fn reconcile(
    manifest: &Manifest,
    state: &mut Value,
    observation: &Observation,
    adapter: &mut impl Adapter,
    at: &str,
) -> io::Result<()> {
    for packet in &manifest.packets {
        let body = state["packets"][packet.number.to_string()]["pending"]["body"].clone();
        let Some(id) = body["id"].as_str() else {
            continue;
        };
        let mut receipt = observation.inboxes[&packet.number]
            .iter()
            .find(|item| item["id"] == id)
            .cloned();
        if receipt.is_none() {
            let reply = adapter.api(
                &packet.owner_host,
                "GET",
                &format!("/api/session/{}/message/{id}", packet.session_id),
                None,
            )?;
            if reply.status != 404 {
                receipt = Some(reply.checked()?);
            }
        }
        if let Some(receipt) = receipt {
            verify_input(&receipt, &body, packet)?;
            acknowledge(state, packet.number, id, at).map_err(invalid)?;
            adapter.persist(state)?;
        }
    }
    Ok(())
}

fn compact(
    packet: &Packet,
    state: &mut Value,
    observation: &Observation,
    adapter: &mut impl Adapter,
    at: &str,
) -> io::Result<bool> {
    let key = packet.number.to_string();
    if state["packets"][&key]["pendingCompaction"].is_null() {
        if observation.inboxes[&packet.number]
            .iter()
            .any(|item| item["type"] == "compact")
        {
            return Ok(true);
        }
        let physical = observation.messages[&packet.number]
            .iter()
            .find(|m| m["type"] == "assistant" && m["id"].is_string())
            .ok_or_else(|| invalid("missing physical context identity"))?;
        let id = format!(
            "msg_{}",
            &format!(
                "{:x}",
                Sha256::digest(
                    format!(
                        "{}{}context-checkpoint",
                        packet.session_id,
                        physical["id"].as_str().expect("validated ID")
                    )
                    .as_bytes()
                )
            )[..32]
        );
        state["packets"][&key]["pendingCompaction"] = json!({"body":{"id":id,"delivery":"steer"},"physicalMessageID":physical["id"],"preparedAt":at});
        adapter.persist(state)?;
    }
    let body = state["packets"][&key]["pendingCompaction"]["body"].clone();
    let id = body["id"]
        .as_str()
        .filter(|id| {
            id.starts_with("msg_")
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        })
        .ok_or_else(|| invalid("invalid durable compaction ID"))?;
    if let Some(item) = observation.inboxes[&packet.number]
        .iter()
        .find(|item| item["id"] == id)
    {
        if item["type"] != "compact" {
            return Err(invalid("compaction inbox receipt type differs"));
        }
        return Ok(true);
    }
    let reply = adapter.api(
        &packet.owner_host,
        "GET",
        &format!("/api/session/{}/message/{id}", packet.session_id),
        None,
    )?;
    if reply.status != 404 {
        let message = reply.checked()?;
        if message["id"] != id || message["type"] != "compaction" {
            return Err(invalid("compaction transcript receipt differs"));
        }
        if message["status"] == "failed" {
            return Err(invalid(
                "native compaction failed; retain its exact receipt for recovery",
            ));
        }
        if message["status"] == "completed" {
            if !state["packets"][&key]["compactionReceipts"].is_object() {
                state["packets"][&key]["compactionReceipts"] = json!({});
            }
            state["packets"][&key]["compactionReceipts"][id] =
                json!({"status":"completed","at":at});
            state["packets"][&key]["pendingCompaction"] = Value::Null;
            adapter.persist(state)?;
            return Ok(false);
        }
        return Ok(true);
    }
    let receipt = adapter
        .api(
            &packet.owner_host,
            "POST",
            &format!("/api/session/{}/compact", packet.session_id),
            Some(&body),
        )?
        .checked()?;
    if receipt["id"] != id
        || receipt["type"] != "compact"
        || receipt["sessionID"] != packet.session_id
    {
        return Err(invalid("compaction submission receipt differs"));
    }
    Ok(true)
}

/// A bounded cycle report; errors and waiting owners never imply acceptance.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CycleReport {
    /// Actual foreground execution.
    pub active: Activity,
    /// Foreground and queued capacity reservations.
    pub occupied: Activity,
    /// Original owners selected this cycle.
    pub selected: Vec<u32>,
    /// Ambiguous or unsuccessful submissions, retained for the next cycle.
    pub errors: Vec<String>,
    /// Last observed context sizes.
    pub context_tokens: BTreeMap<u32, u64>,
    /// Whether this was an authorized writer cycle.
    pub dispatch_enabled: bool,
}

/// Observe, reconcile and admit one bounded slice. The caller owns the writer lease
/// and source refresh. A read-only cycle cannot write state or dispatch inputs.
#[allow(clippy::too_many_arguments)]
pub fn cycle(
    manifest: &Manifest,
    policy: &Policy,
    expected: &Expectations,
    state: &mut Value,
    adapter: &mut impl Adapter,
    at: &str,
    now: i64,
    dispatch: bool,
) -> io::Result<CycleReport> {
    let mut candidate = state.clone();
    for packet in &manifest.packets {
        let ps = super::journal_packet(&mut candidate, packet.number).map_err(invalid)?;
        if !ps["version"].is_string() {
            return Err(invalid(
                "refresh the complete journal before observation/admission",
            ));
        }
    }
    let mut observation = observe(manifest, policy, expected, adapter)?;
    if dispatch {
        reconcile(manifest, &mut candidate, &observation, adapter, at)?;
        for packet in &manifest.packets {
            if observation.occupied[&packet.owner_host].contains(&packet.number) {
                continue;
            }
            let ps = &mut candidate["packets"][packet.number.to_string()];
            if !ps.is_object() {
                return Err(invalid("missing refreshed packet journal"));
            }
            let recovery = observation.messages[&packet.number]
                .iter()
                .find(|m| m["type"] == "idle")
                .is_some_and(|idle| record_idle_recovery(ps, idle, now));
            let recheck = schedule_recheck(ps, policy, now);
            // Recovery/recheck identities enter the event exactly once.
            if recovery || recheck {
                let mut inputs = ps["versionInputs"].clone();
                if !inputs.is_object() {
                    return Err(invalid("refresh source versions before bounded recovery"));
                }
                inputs["progressVersion"] = ps["progressVersion"].clone();
                for key in ["recoveryVersion", "recheckVersion"] {
                    if super::present(&ps[key]) {
                        inputs[key] = ps[key].clone();
                    }
                }
                ps["version"] = json!(digest(&inputs));
            }
        }
    }
    let mut selected = Vec::new();
    let mut reserved = observation.occupied.clone();
    // Exact persisted inputs, absent from both inbox and transcript, use the same
    // host capacity budget as new inputs. They cannot be replaced by a new event.
    for packet in &manifest.packets {
        let ps = &candidate["packets"][packet.number.to_string()];
        if (ps["pending"].is_object() || ps["pendingCompaction"].is_object())
            && !reserved[&packet.owner_host].contains(&packet.number)
            && reserved[&packet.owner_host].len() < policy.max_active_per_host[&packet.owner_host]
        {
            selected.push(packet);
            reserved
                .get_mut(&packet.owner_host)
                .expect("validated host")
                .insert(packet.number);
        }
    }
    selected.extend(select(manifest, &candidate["packets"], &reserved, policy).map_err(invalid)?);
    let mut errors = Vec::new();
    if dispatch {
        adapter.persist(&candidate)?;
        *state = candidate;
        let mut compacting = BTreeSet::new();
        for packet in &manifest.packets {
            let pending =
                state["packets"][packet.number.to_string()]["pendingCompaction"].is_object();
            let admitted = observation.active[&packet.owner_host].contains(&packet.number)
                || selected.iter().any(|p| p.number == packet.number);
            let queued =
                observation.inboxes[&packet.number].iter().any(|item| {
                    item["id"]
                        == state["packets"][packet.number.to_string()]["pendingCompaction"]["body"]
                            ["id"]
                });
            if (pending && (admitted || queued))
                || (admitted && observation.context_tokens[&packet.number] >= 180_000)
            {
                match compact(packet, state, &observation, adapter, at) {
                    Ok(false) => {}
                    outcome => {
                        compacting.insert(packet.number);
                        observation
                            .occupied
                            .get_mut(&packet.owner_host)
                            .expect("validated host")
                            .insert(packet.number);
                        if let Err(error) = outcome {
                            errors.push(format!("owner {} compaction: {error}", packet.number));
                        }
                    }
                }
            }
        }
        for packet in &selected {
            if compacting.contains(&packet.number) {
                continue;
            }
            let ps = &state["packets"][packet.number.to_string()];
            if !ps["pending"].is_object() && !super::needs_wake(ps) {
                continue;
            }
            let result = (|| {
                let text = if state["packets"][packet.number.to_string()]["pending"].is_object() {
                    String::new()
                } else {
                    adapter.continuation(packet, state)?
                };
                let body = prepare(state, packet, policy, &text, at).map_err(invalid)?;
                adapter.persist(state)?;
                // Capacity is reserved even when the mutation's response is lost.
                observation
                    .occupied
                    .get_mut(&packet.owner_host)
                    .expect("validated host")
                    .insert(packet.number);
                let receipt = adapter
                    .api(
                        &packet.owner_host,
                        "POST",
                        &format!("/api/session/{}/prompt", packet.session_id),
                        Some(&body),
                    )?
                    .checked()?;
                if receipt["id"] != body["id"] || receipt["sessionID"] != packet.session_id {
                    return Err(invalid("submission acknowledgement differs"));
                }
                Ok(())
            })();
            if let Err(error) = result {
                errors.push(format!("owner {}: {error}", packet.number));
            }
        }
        adapter.persist(state)?;
    }
    Ok(CycleReport {
        active: observation.active,
        occupied: observation.occupied,
        selected: selected.iter().map(|p| p.number).collect(),
        errors,
        context_tokens: observation.context_tokens,
        dispatch_enabled: dispatch,
    })
}
