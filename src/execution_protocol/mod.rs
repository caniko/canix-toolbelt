//! Provider-free execution protocol. Structural validation grants no authority.
//!
//! The checked-in schema owns both languages' type inventories and constraints.
//! Frames are compact, key-sorted JSON documents (transport delimiters excluded).
//! This rejects duplicate keys and alternate encodings before interpreting IDs.
//! No function here launches a process, connects to an engine, or grants admission.

mod types;
pub use types::*;

use serde_json::Value;
use std::{fmt, sync::OnceLock};

/// The single source for the wire inventory, limits, and generated bindings.
pub const SCHEMA: &str = include_str!("../../runtime/execution/protocol.json");

/// Redacted protocol failure; neither raw input nor credentials enter diagnostics.
#[derive(Debug, Eq, PartialEq)]
pub struct ProtocolError {
    /// Machine-readable failure class.
    pub code: ErrorCode,
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "execution protocol rejected: {:?}", self.code)
    }
}

impl std::error::Error for ProtocolError {}

fn rejected(code: ErrorCode) -> ProtocolError {
    ProtocolError { code }
}

fn schema() -> &'static Value {
    static SCHEMA_VALUE: OnceLock<Value> = OnceLock::new();
    SCHEMA_VALUE.get_or_init(|| {
        serde_json::from_str(SCHEMA).expect("checked-in execution schema must be valid JSON")
    })
}

fn canonical(value: &Value) -> Value {
    match value {
        Value::Object(fields) => {
            let mut entries: Vec<_> = fields.iter().collect();
            entries.sort_unstable_by(|a, b| a.0.cmp(b.0));
            Value::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (key.clone(), canonical(value)))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.iter().map(canonical).collect()),
        _ => value.clone(),
    }
}

fn canonical_numbers(value: &Value) -> bool {
    match value {
        Value::Number(number) => number
            .as_u64()
            .is_some_and(|number| number <= schema()["maxJsonInteger"].as_u64().unwrap_or(0)),
        Value::Object(fields) => fields.values().all(canonical_numbers),
        Value::Array(items) => items.iter().all(canonical_numbers),
        _ => true,
    }
}

fn matches(value: &Value, definition: &Value) -> bool {
    if let Some(name) = definition.as_str() {
        return if name == "bool" {
            value.is_boolean()
        } else {
            schema()["definitions"]
                .get(name)
                .is_some_and(|definition| matches(value, definition))
        };
    }
    if let Some(constant) = definition.get("const") {
        return value == constant;
    }
    if definition.get("type").and_then(Value::as_str) == Some("string") {
        return value
            .as_str()
            .is_some_and(|text| match definition["format"].as_str() {
                Some("opaque") => {
                    !text.is_empty()
                        && text.len() <= definition["maxBytes"].as_u64().unwrap_or(0) as usize
                        && !text.chars().any(char::is_control)
                }
                Some("sha256") | Some("git-revision") => {
                    let length = if definition["format"] == "sha256" {
                        64
                    } else {
                        40
                    };
                    text.len() == length
                        && text
                            .bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                }
                Some("uint64") | Some("generation") => text.parse::<u64>().is_ok_and(|number| {
                    number.to_string() == text
                        && (definition["format"] != "generation" || number > 0)
                }),
                _ => false,
            });
    }
    if let Some(choices) = definition.get("enum").and_then(Value::as_array) {
        return choices.contains(value);
    }
    if let Some(fields) = definition.get("fields").and_then(Value::as_object) {
        return value.as_object().is_some_and(|object| {
            object.len() == fields.len()
                && fields.iter().all(|(name, field)| {
                    object.get(name).is_some_and(|value| matches(value, field))
                })
        });
    }
    if let Some(variants) = definition.get("variants").and_then(Value::as_object) {
        let Some(tag) = definition["tag"].as_str() else {
            return false;
        };
        let Some(mut fields) = value.as_object().cloned() else {
            return false;
        };
        let Some(kind) = fields.remove(tag) else {
            return false;
        };
        return kind
            .as_str()
            .and_then(|kind| variants.get(kind))
            .is_some_and(|variant| matches(&Value::Object(fields), variant));
    }
    if let Some(item) = definition.get("items") {
        return value.as_array().is_some_and(|items| {
            items.len() <= definition["maxItems"].as_u64().unwrap_or(0) as usize
                && items.iter().all(|value| matches(value, item))
                && (definition["unique"] != true
                    || items
                        .iter()
                        .enumerate()
                        .all(|(index, value)| !items[..index].contains(value)))
        });
    }
    false
}

fn validate(value: &Value) -> Result<(), ProtocolError> {
    if value.get("schemaVersion") != Some(&schema()["version"]) {
        return Err(rejected(ErrorCode::IncompatibleVersion));
    }
    if !matches(value, &schema()["root"]) {
        return Err(rejected(ErrorCode::InvalidFrame));
    }
    Ok(())
}

/// Decode a canonical bounded message. Transport must bound reads before calling.
pub fn decode(bytes: &[u8]) -> Result<Message, ProtocolError> {
    if bytes.len() > schema()["maxFrameBytes"].as_u64().unwrap_or(0) as usize {
        return Err(rejected(ErrorCode::InvalidFrame));
    }
    let value: Value =
        serde_json::from_slice(bytes).map_err(|_| rejected(ErrorCode::InvalidFrame))?;
    if !canonical_numbers(&value) {
        return Err(rejected(ErrorCode::InvalidFrame));
    }
    let encoded =
        serde_json::to_vec(&canonical(&value)).map_err(|_| rejected(ErrorCode::InvalidFrame))?;
    if encoded != bytes {
        return Err(rejected(ErrorCode::InvalidFrame));
    }
    validate(&value)?;
    serde_json::from_value(value).map_err(|_| rejected(ErrorCode::InvalidFrame))
}

/// Encode a message after validating every identity, counter, and schema version.
pub fn encode(message: &Message) -> Result<Vec<u8>, ProtocolError> {
    let value = serde_json::to_value(message).map_err(|_| rejected(ErrorCode::InvalidFrame))?;
    validate(&value)?;
    let bytes =
        serde_json::to_vec(&canonical(&value)).map_err(|_| rejected(ErrorCode::InvalidFrame))?;
    if bytes.len() > schema()["maxFrameBytes"].as_u64().unwrap_or(0) as usize {
        return Err(rejected(ErrorCode::InvalidFrame));
    }
    Ok(bytes)
}

/// Match discovery to the caller's exact implementation and required guarantees.
/// This is structural negotiation only; an authenticated custodian must verify
/// the implementation and advertised capabilities before any execution effect.
pub fn negotiate(
    request: &Message,
    response: &Message,
    required: &[Capability],
) -> Result<(), ProtocolError> {
    encode(request)?;
    encode(response)?;
    let (Body::Describe(describe), Body::Capabilities(capabilities)) =
        (&request.body, &response.body)
    else {
        return Err(rejected(ErrorCode::InvalidFrame));
    };
    if request.request_id != response.request_id {
        return Err(rejected(ErrorCode::TransportFailed));
    }
    if capabilities.implementation != describe.expected_implementation
        || describe.expected_implementation.schema_digest != SCHEMA_DIGEST
    {
        return Err(rejected(ErrorCode::CustodyLost));
    }
    if describe.required_profile == Profile::MacosUntrustedReview
        || !capabilities.profiles.contains(&describe.required_profile)
    {
        return Err(rejected(ErrorCode::UnsupportedProfile));
    }
    let mandatory = [
        Capability::AuthenticatedIpc,
        Capability::DurableIntents,
        Capability::ZeroEffectReopen,
        Capability::DescendantCleanup,
    ];
    if mandatory
        .iter()
        .chain(required)
        .any(|capability| !capabilities.capabilities.contains(capability))
    {
        return Err(rejected(ErrorCode::UnsupportedCapability));
    }
    Ok(())
}
