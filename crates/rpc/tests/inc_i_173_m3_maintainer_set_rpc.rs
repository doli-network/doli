//! INC-I-173 M3 — ITEM 2 / spec F6: `getMaintainerSet` must publish the
//! maintainer-set digest and the genesis hash, in EVERY branch, without dropping
//! a single field it already publishes.
//!
//! Closes the AUDIT-P1-003 minimum obligation on the observability side: one
//! scalar per node that an operator can compare across the fleet to answer "do we
//! hold the same release-verification trust root?" without diffing member lists.
//!
//! TDD RED. This file does NOT compile against the tree at `32e0a650`:
//! `doli_core::maintainer::maintainer_set_digest` does not exist. Once it does,
//! the field assertions FAIL at runtime until `get_maintainer_set`
//! (`crates/rpc/src/methods/governance.rs:88`) is wired.
//!
//! Contract: `docs/.workflow/inc-i-173-M3-design-contract.md` Item 2.
//!
//! ---------------------------------------------------------------------------
//! WHY AN INTEGRATION TEST AND NOT AN INLINE `#[cfg(test)]` MODULE
//! ---------------------------------------------------------------------------
//! `get_maintainer_set` is `pub(super) async fn`, so the sibling-module idiom
//! used by `crates/rpc/src/methods/tests_oracle.rs` would require editing
//! `governance.rs` to add a `#[path] mod tests;` declaration — a change to an
//! implementation file, which the M3 test phase may not make. `handle_request`
//! IS `pub` (`crates/rpc/src/methods/dispatch.rs:12`), so this file drives the
//! method exactly as a client does: through the dispatcher, over the real
//! JSON-RPC envelope. That is strictly closer to what an operator observes.
//!
//! ---------------------------------------------------------------------------
//! OUTPUT CONTRACT — `RpcContext::handle_request({method: "getMaintainerSet"})`
//! ---------------------------------------------------------------------------
//! ENUMERATION OF OBSERVABLE OUTPUTS
//!   O1: `JsonRpcResponse.result` PRESENCE (this method never errors).
//!   O2: the set of KEYS in the result object. A REGRESSION output: the CLI at
//!       `bins/cli/src/cmd_governance.rs:466` consumes `maintainers[].pubkey`
//!       and `threshold`, and the operator runbook compares `last_change_block`
//!       across nodes. Adding fields must not remove any.
//!   O3: `result.maintainer_set_digest` — 64 lowercase hex characters, equal to
//!       `maintainer_set_digest(set, genesis_hash_bytes)`.
//!   O4: `result.genesis_hash` — 64 lowercase hex characters, equal to the
//!       node's `ConsensusParams.genesis_hash`.
//!   O5: `result.source` / `result.enforced` — the branch discriminator, which
//!       must keep its current meaning.
//!   mutable params   : NONE. `handle_request(&self, ..)` takes `&self`; the
//!                      maintainer state and chain state are read under
//!                      `RwLock::read`.
//!   persistent store : NONE. This method performs no write.
//!   side channels    : one `debug!` per dispatch. DECLARED UNASSERTED.
//!
//! CODE PATHS (`governance.rs:88-188`)
//!   PB1: `maintainer_state` attached          -> `source: "on-chain"`
//!   PB2: no `maintainer_state`, producer set  -> `source: "derived"` (advisory)
//!   PB3: neither                              -> `source: "none"`
//!   The contract names PB1 and PB2 explicitly. PB3 is covered because it is the
//!   branch that returns the SHORTEST object, and a "just add the fields to the
//!   two big json! blocks" implementation silently leaves it inconsistent.
//!
//! INPUT PARTITIONS
//!   IP-S0 on-chain set EMPTY (a defaulted / never-bootstrapped MaintainerState)
//!   IP-S1 on-chain set of FIVE members, threshold 3, last_updated 172_000
//!   IP-S2 the SAME five members in a DIFFERENT insertion order (the digest must
//!         match IP-S1 through the RPC, not only in the leaf function)
//!   IP-S3 on-chain set of four members (a set that has been rotated)
//!   IP-S4 the SAME five members and threshold with a DIFFERENT `last_updated`
//!         (review iteration 1 / F2 — the digest must MATCH IP-S1 while
//!         `last_change_block` differs; the two values measured on the live
//!         testnet fleet are used)
//! MATRIX: (O1..O5) x {PB1,PB2,PB3} x {IP-S0..IP-S4}.

use std::sync::Arc;

use crypto::{Hash, PublicKey};
use doli_core::consensus::ConsensusParams;
use doli_core::maintainer::{maintainer_set_digest, MaintainerSet};
use doli_core::network::Network;
use mempool::{Mempool, MempoolPolicy};
use rpc::types::JsonRpcRequest;
use rpc::RpcContext;
use serde_json::Value;
use storage::{BlockStore, ChainState, MaintainerState, ProducerSet, UtxoSet};
use tempfile::TempDir;
use tokio::sync::RwLock;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

struct Harness {
    ctx: RpcContext,
    params: ConsensusParams,
    _tempdir: TempDir,
}

fn key(seed: u8) -> PublicKey {
    *crypto::KeyPair::from_seed([seed; 32]).public_key()
}

fn five_members() -> Vec<PublicKey> {
    vec![key(11), key(22), key(33), key(44), key(55)]
}

/// Build an `RpcContext` on `network`, optionally attaching a `MaintainerState`
/// and/or a `ProducerSet` so each of PB1/PB2/PB3 is reachable.
fn harness(
    network: Network,
    maintainer: Option<MaintainerSet>,
    with_producer_set: bool,
) -> Harness {
    let tempdir = TempDir::new().expect("tempdir");
    let params = ConsensusParams::for_network(network);
    let chain_state = Arc::new(RwLock::new(ChainState::new(Hash::ZERO)));
    let utxo_set = Arc::new(RwLock::new(UtxoSet::new()));
    let block_store = Arc::new(BlockStore::open(tempdir.path()).expect("blockstore"));
    let mempool = Arc::new(RwLock::new(Mempool::new(
        MempoolPolicy::default(),
        params.clone(),
        network,
    )));

    let mut ctx = RpcContext::new_for_network(
        chain_state,
        block_store,
        utxo_set,
        mempool,
        params.clone(),
        network,
    );

    if let Some(set) = maintainer {
        ctx.maintainer_state = Some(Arc::new(RwLock::new(MaintainerState {
            version: storage::MAINTAINER_STATE_VERSION,
            set,
            last_derived_height: 1,
        })));
    }
    if with_producer_set {
        ctx = ctx.with_producer_set(Arc::new(RwLock::new(ProducerSet::new())));
    }

    Harness {
        ctx,
        params,
        _tempdir: tempdir,
    }
}

async fn call(h: &Harness) -> Value {
    let response = h
        .ctx
        .handle_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "getMaintainerSet".to_string(),
            params: Value::Null,
            id: Value::from(1),
        })
        .await;
    assert!(
        response.error.is_none(),
        "O1: getMaintainerSet must never return a JSON-RPC error; got {:?}",
        response.error
    );
    response
        .result
        .expect("O1: getMaintainerSet must return a result")
}

fn hex64(v: &Value, field: &str, branch: &str) -> String {
    let s = v
        .get(field)
        .unwrap_or_else(|| {
            panic!(
                "O3/O4 / AUDIT-P1-003: the `{}` branch of getMaintainerSet must \
                 carry a `{}` field. Response was: {}",
                branch, field, v
            )
        })
        .as_str()
        .unwrap_or_else(|| {
            panic!(
                "`{}` must be a JSON string in the `{}` branch",
                field, branch
            )
        })
        .to_string();
    assert_eq!(
        s.len(),
        64,
        "`{}` must be 64 hex characters in the `{}` branch; got {:?}",
        field,
        branch,
        s
    );
    assert!(
        s.chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "`{}` must be LOWERCASE hex in the `{}` branch (two nodes are compared by \
         string equality; a case difference is a false mismatch); got {:?}",
        field,
        branch,
        s
    );
    s
}

// ===========================================================================
// PB1 — the ON-CHAIN branch
// ===========================================================================

/// AUDIT-P1-003 (Must) — the on-chain branch publishes a digest that MATCHES the
/// leaf function over the node's own set and genesis hash.
///
/// Driven over IP-S0/IP-S1/IP-S3 so the assertion covers an empty set, a full
/// five and a rotated four.
#[tokio::test]
async fn audit_p1_003_on_chain_branch_publishes_the_matching_digest() {
    let cases = [
        ("IP-S0 empty", MaintainerSet::new()),
        (
            "IP-S1 five",
            MaintainerSet {
                members: five_members(),
                threshold: 3,
                last_updated: 172_000,
            },
        ),
        (
            "IP-S3 rotated four",
            MaintainerSet {
                members: five_members()[..4].to_vec(),
                threshold: 3,
                last_updated: 172_050,
            },
        ),
    ];

    for (label, set) in cases {
        let h = harness(Network::Mainnet, Some(set.clone()), false);
        let v = call(&h).await;

        assert_eq!(
            v.get("source").and_then(Value::as_str),
            Some("on-chain"),
            "O5 ({}): setup must reach the on-chain branch",
            label
        );

        let genesis_hex = hex64(&v, "genesis_hash", "on-chain");
        assert_eq!(
            genesis_hex,
            h.params.genesis_hash.to_hex(),
            "O4 ({}): `genesis_hash` must be the node's own ConsensusParams \
             genesis hash — it is what makes the digest a per-CHAIN answer, so two \
             chains with identical member lists cannot report the same digest",
            label
        );

        let digest_hex = hex64(&v, "maintainer_set_digest", "on-chain");
        assert_eq!(
            digest_hex,
            hex::encode(maintainer_set_digest(
                &set,
                h.params.genesis_hash.as_bytes()
            )),
            "O3 ({}): the published digest must equal \
             maintainer_set_digest(state.set, genesis_hash). A digest computed \
             over anything else cannot be compared against another node's.",
            label
        );
    }
}

/// AUDIT-P1-003 (Must) — the on-chain branch keeps EVERY field it publishes
/// today.
///
/// Regression output O2. The field list is read from
/// `crates/rpc/src/methods/governance.rs:105-117` at the M3 branch point.
/// `bins/cli/src/cmd_governance.rs:466` consumes `maintainers[].pubkey` and
/// `threshold`; the operator runbook compares `last_change_block`. F6 ADDS
/// fields — it must remove none.
#[tokio::test]
async fn audit_p1_003_on_chain_branch_keeps_every_existing_field() {
    let set = MaintainerSet {
        members: five_members(),
        threshold: 3,
        last_updated: 172_000,
    };
    let h = harness(Network::Mainnet, Some(set), false);
    let v = call(&h).await;

    for field in [
        "maintainers",
        "threshold",
        "member_count",
        "max_maintainers",
        "min_maintainers",
        "initial_maintainer_count",
        "last_change_block",
        "source",
        "enforced",
        "maintainer_derivation_activation_height",
    ] {
        assert!(
            v.get(field).is_some(),
            "O2: getMaintainerSet dropped the pre-existing field `{}` from the \
             on-chain branch. F6 adds two fields; it removes none. Response: {}",
            field,
            v
        );
    }

    let members = v
        .get("maintainers")
        .and_then(Value::as_array)
        .expect("O2: `maintainers` must stay an array");
    assert_eq!(members.len(), 5, "O2: all five members must be listed");
    for m in members {
        assert!(
            m.get("pubkey").and_then(Value::as_str).is_some(),
            "O2: each maintainer entry must keep its `pubkey` field — \
             cmd_governance.rs:466 reads it"
        );
    }
    assert_eq!(
        v.get("last_change_block").and_then(Value::as_u64),
        Some(172_000),
        "O2: `last_change_block` must still report `set.last_updated`. F6 is \
         additive — every field this method already published must survive, or the \
         digest was bought by breaking an existing consumer."
    );
    assert_eq!(
        v.get("enforced").and_then(Value::as_bool),
        Some(true),
        "O5: the on-chain branch is the enforced root"
    );
}

/// AUDIT-P1-003 (Must) — through the RPC, member INSERTION ORDER does not change
/// the published digest.
///
/// The leaf function is order-independent (asserted in
/// `crates/core/tests/inc_i_173_m3_maintainer_digest.rs`); this pins that the RPC
/// does not undo that property by, for example, digesting the already-serialized
/// `maintainers` array instead of the set.
#[tokio::test]
async fn audit_p1_003_published_digest_is_independent_of_member_order() {
    let mut reversed = five_members();
    reversed.reverse();

    let a = harness(
        Network::Mainnet,
        Some(MaintainerSet {
            members: five_members(),
            threshold: 3,
            last_updated: 172_000,
        }),
        false,
    );
    let b = harness(
        Network::Mainnet,
        Some(MaintainerSet {
            members: reversed,
            threshold: 3,
            last_updated: 172_000,
        }),
        false,
    );

    assert_eq!(
        hex64(&call(&a).await, "maintainer_set_digest", "on-chain"),
        hex64(&call(&b).await, "maintainer_set_digest", "on-chain"),
        "O3: two nodes holding the SAME five keys in different insertion order \
         must publish the SAME digest. An order-sensitive digest sends an operator \
         chasing a divergence that does not exist."
    );
}

/// AUDIT-P1-003 / F2 (Must) — IP-S4: two nodes that hold the SAME trust root but
/// disagree on `last_change_block` publish the SAME digest.
///
/// This is the exact fleet shape the M3 security audit MEASURED
/// (`docs/.workflow/chain-state.md:36-39`): RPC 8512 reported
/// `last_change_block = 88289` while 12 peers reported `1`, all at tip 134,682,
/// all holding the same five members and the same threshold. Those 13 nodes
/// accept identical release signatures, so an operator comparing digests must
/// see a MATCH. Before F2 the digest bound `last_updated` and reported a
/// mismatch for this aligned fleet, which is worse than publishing no digest:
/// an instrument that cries wolf gets ignored.
///
/// The test also asserts the operator does not LOSE the history term —
/// `last_change_block` must still differ between the two responses.
#[tokio::test]
async fn audit_p1_003_published_digest_is_independent_of_last_change_block() {
    let divergent = harness(
        Network::Mainnet,
        Some(MaintainerSet {
            members: five_members(),
            threshold: 3,
            last_updated: 88_289,
        }),
        false,
    );
    let peers = harness(
        Network::Mainnet,
        Some(MaintainerSet {
            members: five_members(),
            threshold: 3,
            last_updated: 1,
        }),
        false,
    );

    let dv = call(&divergent).await;
    let pv = call(&peers).await;

    assert_eq!(
        hex64(&dv, "maintainer_set_digest", "on-chain"),
        hex64(&pv, "maintainer_set_digest", "on-chain"),
        "O3 / F2: two nodes holding the SAME five members and the SAME threshold \
         must publish the SAME digest even when `last_change_block` differs. That \
         fleet was MEASURED on the live testnet (8512 at 88289 vs 12 peers at 1, \
         identical tip); it is aligned on its release-verification root, and the \
         digest's stated question is exactly that."
    );

    assert_ne!(
        dv.get("last_change_block").and_then(Value::as_u64),
        pv.get("last_change_block").and_then(Value::as_u64),
        "O2 / F2: excluding `last_updated` from the DIGEST must not remove it from \
         the RESPONSE — `last_change_block` is still published verbatim, so the \
         operator keeps the history term and gains a comparison scalar that does \
         not false-alarm"
    );
    assert_eq!(
        dv.get("last_change_block").and_then(Value::as_u64),
        Some(88_289),
        "O2: `last_change_block` must report `set.last_updated` verbatim"
    );
}

/// AUDIT-P1-003 (Must) — the digest DIFFERS across networks for the same set.
///
/// `bootstrap_maintainer_keys` is byte-identical for mainnet and testnet
/// (`crates/updater/src/constants.rs:53-86`), so without `genesis_hash` in the
/// preimage an operator comparing a mainnet node against a testnet node sees a
/// false MATCH on the bootstrap five.
#[tokio::test]
async fn audit_p1_003_published_digest_differs_between_mainnet_and_testnet() {
    let set = MaintainerSet {
        members: five_members(),
        threshold: 3,
        last_updated: 172_000,
    };
    let m = harness(Network::Mainnet, Some(set.clone()), false);
    let t = harness(Network::Testnet, Some(set), false);

    let mv = call(&m).await;
    let tv = call(&t).await;

    assert_ne!(
        hex64(&mv, "genesis_hash", "on-chain"),
        hex64(&tv, "genesis_hash", "on-chain"),
        "fixture: mainnet and testnet must report different genesis hashes"
    );
    assert_ne!(
        hex64(&mv, "maintainer_set_digest", "on-chain"),
        hex64(&tv, "maintainer_set_digest", "on-chain"),
        "O3: the SAME maintainer set on mainnet and testnet must publish DIFFERENT \
         digests"
    );
}

// ===========================================================================
// PB2 — the ADVISORY / `derived` fallback branch
// ===========================================================================

/// AUDIT-P1-003 (Must) — the advisory-fallback branch carries BOTH new fields.
///
/// The contract names this branch explicitly. It is the easy one to forget:
/// there are two large `json!` blocks in `get_maintainer_set` and only the first
/// is on the obvious path.
#[tokio::test]
async fn audit_p1_003_derived_branch_publishes_digest_and_genesis_hash() {
    let h = harness(Network::Mainnet, None, true);
    let v = call(&h).await;

    assert_eq!(
        v.get("source").and_then(Value::as_str),
        Some("derived"),
        "setup must reach the advisory-fallback branch; got {}",
        v
    );

    let genesis_hex = hex64(&v, "genesis_hash", "derived");
    assert_eq!(
        genesis_hex,
        h.params.genesis_hash.to_hex(),
        "O4: the advisory branch must report the same genesis hash as the on-chain \
         branch — it is a property of the CHAIN, not of which branch answered"
    );

    // The advisory branch reports the CANONICALLY DERIVED set. With an empty
    // producer set that derivation yields an empty membership with
    // `calculate_threshold(0)`, and the digest must be computed over exactly
    // that value — not over a defaulted or zeroed placeholder.
    let threshold = v
        .get("threshold")
        .and_then(Value::as_u64)
        .expect("O2: the advisory branch must keep `threshold`") as usize;
    let last_change = v
        .get("last_change_block")
        .and_then(Value::as_u64)
        .expect("O2: the advisory branch must keep `last_change_block`");
    let expected = MaintainerSet {
        members: Vec::new(),
        threshold,
        last_updated: last_change,
    };
    assert_eq!(
        hex64(&v, "maintainer_set_digest", "derived"),
        hex::encode(maintainer_set_digest(
            &expected,
            h.params.genesis_hash.as_bytes()
        )),
        "O3: the advisory branch must digest the set it REPORTS. A digest that \
         does not match the reported members/threshold/last_change_block is worse \
         than none: it looks comparable and is not."
    );
    assert_eq!(
        v.get("enforced").and_then(Value::as_bool),
        Some(false),
        "O5: the advisory branch must stay `enforced: false` — the digest does not \
         promote it to an enforced root"
    );
}

/// AUDIT-P1-003 (Must) — the advisory branch keeps every field it publishes
/// today, including `advisory_note`.
///
/// Field list read from `crates/rpc/src/methods/governance.rs:175-187`.
#[tokio::test]
async fn audit_p1_003_derived_branch_keeps_every_existing_field() {
    let h = harness(Network::Mainnet, None, true);
    let v = call(&h).await;

    for field in [
        "maintainers",
        "threshold",
        "member_count",
        "max_maintainers",
        "min_maintainers",
        "initial_maintainer_count",
        "last_change_block",
        "source",
        "enforced",
        "maintainer_derivation_activation_height",
        "advisory_note",
    ] {
        assert!(
            v.get(field).is_some(),
            "O2: getMaintainerSet dropped the pre-existing field `{}` from the \
             advisory branch. Response: {}",
            field,
            v
        );
    }
}

// ===========================================================================
// PB3 — the `none` branch
// ===========================================================================

/// AUDIT-P1-003 (Should) — the `none` branch stays consistent.
///
/// Neither a `MaintainerState` nor a `ProducerSet` is attached, so no maintainer
/// root can be reported. This branch must NOT publish a digest: a digest here
/// would describe nothing, and a consumer comparing it against a real node's
/// would get a meaningless mismatch. It must still carry `genesis_hash`, which
/// is a property of the chain and is always knowable.
#[tokio::test]
async fn audit_p1_003_none_branch_reports_no_digest_but_still_identifies_the_chain() {
    let h = harness(Network::Mainnet, None, false);
    let v = call(&h).await;

    assert_eq!(
        v.get("source").and_then(Value::as_str),
        Some("none"),
        "setup must reach the `none` branch; got {}",
        v
    );
    assert!(
        v.get("maintainer_set_digest").is_none(),
        "O3: the `none` branch has no set to digest, so it must publish NO \
         `maintainer_set_digest`. A digest over an absent set is a value that \
         invites a comparison it cannot support. Response: {}",
        v
    );
    assert_eq!(
        hex64(&v, "genesis_hash", "none"),
        h.params.genesis_hash.to_hex(),
        "O4: the chain identity is always knowable and must be reported even when \
         no maintainer root is"
    );
    assert_eq!(
        v.get("enforced").and_then(Value::as_bool),
        Some(false),
        "O5: unchanged"
    );
}
