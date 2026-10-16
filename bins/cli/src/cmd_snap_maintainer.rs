//! INC-I-248 M1.5 — the maintainer state that `doli snap` installs with the 3-state.

use std::path::Path;

use anyhow::{anyhow, bail, Result};
use crypto::{Hash, PublicKey};
use doli_core::maintainer::{holds_retired_key_at, maintainer_set_digest, MaintainerSet};
use serde_json::Value;
use storage::{validate_persisted_set, MaintainerState, MAINTAINER_STATE_VERSION};

/// What `write_snap_maintainer_state` installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapMaintainerOutcome {
    /// The snapshot's set, verified against the seed-agreed digest.
    Canonical,
    /// EMPTY at the snapshot height; the reason the set was refused.
    FailClosed(String),
}

/// Seed agreement on the snapshot's maintainer digest, the state-root rule of
/// `cmd_snap`: `count` seeds answer `snapshot_digest`; accepted when
/// `trust || count >= 1`, refused when `!trust && seed_count >= 2 && count < 2`.
pub fn agree_maintainer_digest(
    answers: &[(String, String)],
    snapshot_digest: &str,
    seed_count: usize,
    trust: bool,
) -> Option<String> {
    let count = answers.iter().filter(|(_, d)| d == snapshot_digest).count();
    let accepted = (trust || count >= 1) && !(!trust && seed_count >= 2 && count < 2);
    accepted.then(|| snapshot_digest.to_string())
}

/// `getMaintainerSet` digest of each seed that answers.
pub async fn query_maintainer_answers(seeds: &[&str]) -> Vec<(String, String)> {
    let mut answers = Vec::new();
    for seed in seeds {
        match query_maintainer_digest(seed).await {
            Ok(digest) => {
                println!("  {} — maintainer digest={:.16}...", seed, digest);
                answers.push((seed.to_string(), digest));
            }
            Err(e) => println!("  {} — maintainer digest not counted ({})", seed, e),
        }
    }
    answers
}

/// Height above which the retired genesis keys are not maintainers; `u64::MAX`
/// for an unknown network name.
pub fn retired_keys_height(network: &str) -> u64 {
    <doli_core::Network as std::str::FromStr>::from_str(network)
        .map(|n| n.params().retired_genesis_maintainer_keys_height)
        .unwrap_or(u64::MAX)
}

/// Always writes `maintainer_state.bin`: the verified snapshot set, else EMPTY.
/// `last_derived_height` is the snapshot height on both paths.
pub fn write_snap_maintainer_state(
    data_dir: &Path,
    field: &Value,
    height: u64,
    agreed_digest: Option<&str>,
    genesis_hash: &Hash,
    retired_after: u64,
) -> Result<SnapMaintainerOutcome> {
    let verified =
        verified_set(data_dir, field, height, agreed_digest, genesis_hash).and_then(|set| {
            if holds_retired_key_at(&set, height, retired_after) {
                Err(format!(
                    "the set holds a genesis key retired after height {}",
                    retired_after
                ))
            } else {
                Ok(set)
            }
        });
    let (set, outcome) = match verified {
        Ok(set) => (set, SnapMaintainerOutcome::Canonical),
        Err(reason) => (
            MaintainerSet::new(),
            SnapMaintainerOutcome::FailClosed(reason),
        ),
    };
    MaintainerState {
        version: MAINTAINER_STATE_VERSION,
        set,
        last_derived_height: height,
    }
    .save(data_dir)
    .map_err(|e| anyhow!("write maintainer_state.bin: {}", e))?;
    Ok(outcome)
}

fn verified_set(
    data_dir: &Path,
    field: &Value,
    height: u64,
    agreed_digest: Option<&str>,
    genesis_hash: &Hash,
) -> std::result::Result<MaintainerSet, String> {
    let obj = field
        .as_object()
        .ok_or("the snapshot carries no maintainerState object")?;
    let members = obj
        .get("members")
        .and_then(Value::as_array)
        .ok_or("maintainerState.members is missing")?
        .iter()
        .map(|m| {
            m.as_str()
                .and_then(|s| PublicKey::from_hex(s).ok())
                .ok_or_else(|| format!("maintainerState member {} is not a public key", m))
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let threshold = obj
        .get("threshold")
        .and_then(Value::as_u64)
        .and_then(|t| usize::try_from(t).ok())
        .ok_or("maintainerState.threshold is missing")?;
    let last_change = obj
        .get("lastChangeBlock")
        .and_then(Value::as_u64)
        .ok_or("maintainerState.lastChangeBlock is missing")?;
    if last_change > height {
        return Err(format!(
            "lastChangeBlock {} is above the snapshot height {}",
            last_change, height
        ));
    }
    let mut set = MaintainerSet::with_members(members, last_change);
    set.threshold = threshold;
    validate_persisted_set(&data_dir.join("maintainer_state.bin"), &set)
        .map_err(|e| e.to_string())?;

    let digest = hex::encode(maintainer_set_digest(&set, genesis_hash.as_bytes()));
    if obj.get("digest").and_then(Value::as_str) != Some(digest.as_str()) {
        return Err("maintainerState.digest does not match the recomputed digest".into());
    }
    match agreed_digest {
        None => Err("the seeds did not agree on a maintainer digest".into()),
        Some(agreed) if agreed != digest => {
            Err("the snapshot set differs from the seed-agreed digest".into())
        }
        Some(_) => Ok(set),
    }
}

/// A seed's `getMaintainerSet` digest; only an on-chain, non-empty set counts.
pub async fn query_maintainer_digest(rpc_url: &str) -> Result<String> {
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(10))
        .build()?;
    let body: Value = client
        .post(rpc_url)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "getMaintainerSet",
            "params": {},
            "id": 1
        }))
        .send()
        .await?
        .json()
        .await?;

    let r = body.get("result").ok_or_else(|| anyhow!("No result"))?;
    if r["source"].as_str() != Some("on-chain") {
        bail!("maintainer set source is not on-chain");
    }
    if r["member_count"].as_u64().unwrap_or(0) == 0 {
        bail!("maintainer set is empty");
    }
    r["maintainer_set_digest"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| anyhow!("No maintainer_set_digest"))
}
