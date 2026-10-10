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
    /// The response's `data` value. Paginated message listings retain their
    /// complete `{data, cursor}` envelope so bounded context lookup can continue.
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
    /// Recent messages used for idle recovery; physical usage is queried separately.
    pub messages: BTreeMap<u32, Vec<Value>>,
    /// Last observed input/cache tokens per owner.
    pub context_tokens: BTreeMap<u32, u64>,
    /// Exact assistant supplying physical usage, retained across compaction.
    pub physical_message_ids: BTreeMap<u32, String>,
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

fn verify_identity(info: &Value, packet: &Packet, identity: &Identity) -> io::Result<()> {
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
    Ok(())
}

fn verify_inbox(inbox: &[Value]) -> io::Result<()> {
    let mut ids = BTreeSet::new();
    if inbox.iter().any(|item| {
        !item["id"]
            .as_str()
            .is_some_and(|id| !id.is_empty() && ids.insert(id))
    }) {
        return Err(invalid("missing or duplicate inbox receipt identity"));
    }
    Ok(())
}

fn verify_mirror_inboxes(
    adapter: &mut impl Adapter,
    packet: &Packet,
    hosts: impl Iterator<Item = impl AsRef<str>>,
) -> io::Result<()> {
    for host in hosts {
        let host = host.as_ref();
        if host == packet.owner_host {
            continue;
        }
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
    Ok(())
}

enum ContextSearch {
    Continue,
    Found,
    Finished,
}

// V2 filters message types before pagination. Tool/status traffic therefore
// cannot hide physical usage. Exhausting this bounded lookup remains unknown,
// never an implicit zero-token context.
fn context_message(
    adapter: &mut impl Adapter,
    packet: &Packet,
    kind: &str,
    mut select: impl FnMut(&Value) -> io::Result<ContextSearch>,
) -> io::Result<Option<Value>> {
    let mut cursor = None::<String>;
    let mut cursors = BTreeSet::new();
    let mut ids = BTreeSet::new();
    let mut previous_time = u64::MAX;
    for _ in 0..4 {
        let suffix = match &cursor {
            None => format!("/message?limit=64&order=desc&type={kind}"),
            Some(cursor) => {
                let encoded: String = cursor
                    .bytes()
                    .flat_map(|byte| {
                        let hex = b"0123456789ABCDEF";
                        [
                            '%',
                            hex[(byte >> 4) as usize] as char,
                            hex[(byte & 15) as usize] as char,
                        ]
                    })
                    .collect();
                format!("/message?limit=64&type={kind}&cursor={encoded}")
            }
        };
        let page = request(adapter, packet, &suffix)?;
        let messages = page
            .as_array()
            .or_else(|| page["data"].as_array())
            .filter(|items| items.len() <= 64)
            .ok_or_else(|| invalid("malformed filtered context page"))?;
        for message in messages {
            let id = message["id"]
                .as_str()
                .filter(|id| id.starts_with("msg_"))
                .ok_or_else(|| invalid("missing context message identity"))?;
            let time = message["time"]["created"]
                .as_u64()
                .ok_or_else(|| invalid("missing or malformed context message time"))?;
            if message["type"] != kind || time > previous_time || !ids.insert(id.to_owned()) {
                return Err(invalid("incoherent filtered context history"));
            }
            previous_time = time;
            match select(message)? {
                ContextSearch::Found => return Ok(Some(message.clone())),
                ContextSearch::Finished => return Ok(None),
                ContextSearch::Continue => {}
            }
        }
        if page.is_array() {
            if messages.len() == 64 {
                return Err(invalid("context page omitted its continuation cursor"));
            }
            return Ok(None);
        }
        let next = page["cursor"]
            .get("next")
            .ok_or_else(|| invalid("context page omitted its continuation cursor"))?;
        if next.is_null() {
            return Ok(None);
        }
        let next = next
            .as_str()
            .filter(|value| !value.is_empty() && value.len() <= 4096)
            .ok_or_else(|| invalid("malformed context continuation cursor"))?;
        if messages.is_empty() || !cursors.insert(next.to_owned()) {
            return Err(invalid("context pagination made no progress"));
        }
        cursor = Some(next.to_owned());
    }
    Err(invalid(
        "physical context lookup exceeded its bounded history",
    ))
}

fn physical_tokens(message: &Value) -> io::Result<u64> {
    [
        &message["tokens"]["input"],
        &message["tokens"]["cache"]["read"],
        &message["tokens"]["cache"]["write"],
    ]
    .into_iter()
    .try_fold(0u64, |total, value| {
        let tokens = value
            .as_u64()
            .ok_or_else(|| invalid("missing or malformed physical context usage"))?;
        total
            .checked_add(tokens)
            .ok_or_else(|| invalid("physical context usage overflow"))
    })
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
    let mut physical_message_ids = BTreeMap::new();
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
        verify_mirror_inboxes(adapter, packet, raw.keys())?;
        let info = request(adapter, packet, "")?;
        verify_identity(&info, packet, &expected[&packet.number])?;
        let inbox = request(adapter, packet, "/inbox")?
            .as_array()
            .cloned()
            .ok_or_else(|| invalid("incomplete inbox observation"))?;
        verify_inbox(&inbox)?;
        let recent_page = request(adapter, packet, "/message?limit=12&order=desc")?;
        let recent = recent_page
            .as_array()
            .or_else(|| recent_page["data"].as_array())
            .cloned()
            .ok_or_else(|| invalid("incomplete context observation"))?;
        let physical = context_message(adapter, packet, "assistant", |message| {
            Ok(if physical_tokens(message)? > 0 {
                ContextSearch::Found
            } else {
                ContextSearch::Continue
            })
        })?
        .ok_or_else(|| {
            invalid(format!(
                "owner {} physical context is unknown",
                packet.number
            ))
        })?;
        let physical_time = physical["time"]["created"]
            .as_u64()
            .expect("validated time");
        let compacted = context_message(adapter, packet, "compaction", |message| {
            if !matches!(
                message["status"].as_str(),
                Some("completed" | "failed" | "running")
            ) {
                return Err(invalid("malformed context compaction status"));
            }
            Ok(
                if message["time"]["created"].as_u64().expect("validated time") <= physical_time {
                    ContextSearch::Finished
                } else if message["status"] == "completed" {
                    ContextSearch::Found
                } else {
                    ContextSearch::Continue
                },
            )
        })?
        .is_some();
        let tokens = if compacted {
            0
        } else {
            physical_tokens(&physical)?
        };
        if tokens >= 300_000 {
            return Err(invalid(format!(
                "owner {} context ceiling reached: {tokens}",
                packet.number
            )));
        }
        inboxes.insert(packet.number, inbox);
        messages.insert(packet.number, recent);
        context_tokens.insert(packet.number, tokens);
        physical_message_ids.insert(
            packet.number,
            physical["id"].as_str().expect("validated ID").to_owned(),
        );
    }
    // A queue can change without foreground activity changing. Revalidate both
    // owner and mirror inboxes, then bracket the complete pass with activity.
    for packet in &manifest.packets {
        verify_mirror_inboxes(adapter, packet, raw.keys())?;
        let fresh = request(adapter, packet, "/inbox")?;
        let fresh = fresh
            .as_array()
            .ok_or_else(|| invalid("incomplete final inbox observation"))?;
        verify_inbox(fresh)?;
        if fresh != &inboxes[&packet.number] {
            return Err(invalid(format!(
                "owner {} inbox changed during observation",
                packet.number
            )));
        }
    }
    for (host, before) in &raw {
        let after = adapter
            .api(host, "GET", "/api/session/active", None)?
            .checked()?;
        if &after != before {
            return Err(invalid(format!(
                "activity changed during inbox collection on {host}"
            )));
        }
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
        physical_message_ids,
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
    let empty = json!([]);
    for key in ["files", "agents", "skills"] {
        let expected = if body[key].is_null() {
            &empty
        } else {
            &body[key]
        };
        let actual = if payload[key].is_null() {
            &empty
        } else {
            &payload[key]
        };
        if actual != expected {
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
            // Reconciliation changes only the cycle's private candidate. All
            // receipt lookups/validation must succeed before its single commit.
        }
    }
    Ok(())
}

fn compact(
    packet: &Packet,
    identity: &Identity,
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
        let physical = &observation.physical_message_ids[&packet.number];
        let id = format!(
            "msg_{}",
            &format!(
                "{:x}",
                Sha256::digest(
                    format!("{}{}context-checkpoint", packet.session_id, physical).as_bytes()
                )
            )[..32]
        );
        state["packets"][&key]["pendingCompaction"] = json!({"body":{"id":id,"delivery":"steer"},"physicalMessageID":physical,"preparedAt":at});
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
    verify_identity(&request(adapter, packet, "")?, packet, identity)?;
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
                match compact(
                    packet,
                    &expected[&packet.number],
                    state,
                    &observation,
                    adapter,
                    at,
                ) {
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
                verify_identity(
                    &request(adapter, packet, "")?,
                    packet,
                    &expected[&packet.number],
                )?;
                // A retry may occur long after preparation. Bind this actual
                // submission attempt to the exact pending body before POST.
                state["packets"][packet.number.to_string()]["pending"]["submissionAttemptAt"] =
                    json!(at);
                adapter.persist(state)?;
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
