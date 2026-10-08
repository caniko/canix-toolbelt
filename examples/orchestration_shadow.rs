//! Read-only compatibility qualification; never submits inputs or writes journals.
use canix_toolbelt::orchestration::{Manifest, Policy, digest, refresh};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let paths: Vec<_> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    if paths.len() != 4 {
        return Err("usage: orchestration_shadow MANIFEST STATE CONTROL HANDOFFS".into());
    }
    let manifest_bytes = fs::read(&paths[0])?;
    let state_bytes = fs::read(&paths[1])?;
    let manifest: Manifest = serde_json::from_slice(&manifest_bytes)?;
    manifest.validate(None)?;
    let original: Value = serde_json::from_slice(&state_bytes)?;
    let control: Value = serde_json::from_slice(&fs::read(&paths[2])?)?;
    let policy = Policy {
        goal_policy: control["goalPolicy"]
            .as_str()
            .ok_or("missing goalPolicy")?
            .into(),
        max_active_per_host: serde_json::from_value(control["maxActivePerHost"].clone())?,
        producers: manifest
            .packets
            .iter()
            .filter(|packet| packet.extra.get("schedulerProducer") == Some(&json!(true)))
            .map(|packet| packet.number)
            .collect(),
        priority: vec![],
        expedited: serde_json::from_value(control["expeditePackets"].clone()).unwrap_or_default(),
    };
    let mut dependencies = BTreeMap::new();
    let mut reports = BTreeMap::new();
    for packet in &manifest.packets {
        let report = &original["packets"][packet.number.to_string()]["workerStatus"];
        if report.is_object() {
            reports.insert(packet.number, report.clone());
        }
        let path = paths[3].join(format!("{:02}.md", packet.number));
        if path.is_file() {
            dependencies.insert(
                packet.number,
                format!("{:x}", Sha256::digest(fs::read(path)?)),
            );
        }
    }
    let mut candidate = original.clone();
    refresh(
        &manifest,
        &mut candidate,
        &policy,
        &reports,
        &BTreeMap::new(),
        &dependencies,
    )?;
    let changed: Vec<_> = manifest
        .packets
        .iter()
        .filter(|packet| {
            original["packets"][packet.number.to_string()]["version"]
                != candidate["packets"][packet.number.to_string()]["version"]
        })
        .map(|packet| packet.number)
        .collect();
    let pending_preserved = manifest.packets.iter().all(|packet| {
        original["packets"][packet.number.to_string()]["pending"]
            == candidate["packets"][packet.number.to_string()]["pending"]
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "manifestDigest":format!("{:x}",Sha256::digest(&manifest_bytes)),
            "stateBytesDigest":format!("{:x}",Sha256::digest(&state_bytes)),
            "stateDigest":digest(&original), "owners":manifest.packets.len(),
            "assignments":manifest.assignment_count,
            "unchangedEventCount":manifest.packets.len()-changed.len(),
            "changedEventPackets":changed, "pendingBodiesPreserved":pending_preserved,
            "dispatchEnabled":false, "journalWritten":false
        }))?
    );
    if !pending_preserved {
        return Err("shadow altered a persisted pending request".into());
    }
    Ok(())
}
